mod returnable;

pub use returnable::{Return, Returnable};

use futures::stream;
use hyper::Body;
use std::str::FromStr;
use warp::Filter;

use samizdat_common::Hash;

use crate::cache::{ObjectRef, ObjectStream};
use crate::hubs;

fn reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: Returnable,
{
    warp::reply::with_header(
        warp::reply::with_status(t.render().into_owned(), t.status_code()),
        http::header::CONTENT_TYPE,
        &*t.content_type(),
    )
}

async fn async_reply<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

pub fn get_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / String)
        .and(warp::get())
        .and_then(|hash: String| async move {
            // Try get from local:
            let object = ObjectRef::new(Hash::from_str(&hash)?);

            let stream = if let Some(stream) = object.iter()? {
                log::info!("found local hash {}", object.hash);
                Some(stream)
            } else {
                hubs().query(object.hash).await;
                object.iter()?
            };

            // Respond with found or not found.
            if let Some(ObjectStream {
                metadata,
                iter_chunks,
            }) = stream
            {
                let response = http::Response::builder()
                    .header("Content-Type", metadata.content_type)
                    .header("Content-Size", metadata.content_size)
                    .status(http::StatusCode::OK)
                    // TODO: Bleh! Tidy-up this mess!
                    .body(Body::wrap_stream(stream::iter(
                        iter_chunks.map(|thing| thing.map_err(|err| err.to_string())),
                    )));

                Ok(response) as Result<_, warp::Rejection>
            } else {
                let response = http::Response::builder()
                    .header("Content-Type", "text/plain")
                    .status(http::StatusCode::NOT_FOUND)
                    .body(Body::from(""));

                Ok(response)
            }
        })
}

pub fn post_content() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_content")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::body::bytes())
        .map(|content_type: String, bytes: bytes::Bytes| async move {
            let (_, object) = ObjectRef::build(
                content_type,
                bytes.len(),
                stream::iter(bytes.into_iter().map(|byte| Ok(byte))),
            )
            .await?;
            Ok(object.hash.to_string())
        })
        .and_then(async_reply)
}
