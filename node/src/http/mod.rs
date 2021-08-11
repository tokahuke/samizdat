mod returnable;

pub use returnable::{Return, Returnable};

use futures::stream;
use http::Response;
use hyper::Body;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::Hash;

use crate::cache::{CollectionRef, ObjectRef, ObjectStream};
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

async fn get_object(
    object: ObjectRef,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
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

        Ok(response)
    } else {
        let response = http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(""));

        Ok(response)
    }
}

pub fn get_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / Hash)
        .and(warp::get())
        .and_then(|hash: Hash| async move {
            Ok(get_object(ObjectRef::new(hash)).await?) as Result<_, warp::Rejection>
        })
}

pub fn post_content() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash")
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

pub fn delete_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / Hash)
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).drop_if_exists())
        .map(reply)
}

pub fn post_collection() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone
{
    warp::path!("_collection")
        .and(warp::post())
        .and(warp::body::json())
        .map(|hashes: Vec<(String, Hash)>| {
            let collection = CollectionRef::build(
                hashes
                    .into_iter()
                    .map(|(name, hash)| (name, ObjectRef::new(hash)))
                    .collect::<Vec<_>>(),
            )?;
            Ok(Return {
                content_type: "text/plain".to_owned(),
                status_code: http::StatusCode::OK,
                content: collection.hash.0.into(),
            })
        })
        .map(reply)
}

pub fn get_item() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_collection" / Hash / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|hash: Hash, name: Tail| async move {
            let collection = CollectionRef::new(hash);
            let item = collection
                .get(name.as_str().to_owned())?
                .expect("object exists");

            Ok(get_object(item.object).await?) as Result<_, warp::Rejection>
        })
}
