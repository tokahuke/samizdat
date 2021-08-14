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

async fn resolve_object(
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

pub fn get_object() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::get())
        .and_then(|hash: Hash| async move {
            Ok(resolve_object(ObjectRef::new(hash)).await?) as Result<_, warp::Rejection>
        })
}

pub fn post_object() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_objects")
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

pub fn delete_object() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).drop_if_exists())
        .map(reply)
}

pub fn post_collection() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone
{
    warp::path!("_collections")
        .and(warp::post())
        .and(warp::body::json())
        .map(|hashes: Vec<(String, String)>| {
            let collection = CollectionRef::build(
                hashes
                    .into_iter()
                    .map(|(name, hash)| Ok((name, ObjectRef::new(hash.parse()?))))
                    .collect::<Result<Vec<_>, crate::Error>>()?,
            )?;
            Ok(Return {
                content_type: "text/plain".to_owned(),
                status_code: http::StatusCode::OK,
                content: collection.hash.to_string().as_bytes().to_vec(),
            })
        })
        .map(reply)
}

pub fn get_item() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_collections" / Hash / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|hash: Hash, name: Tail| async move {
            let collection = CollectionRef::new(hash);
            if let Some(item) = collection.get(name.as_str())? {
                assert!(item.inclusion_proof.is_in(&hash));
                Ok(resolve_object(item.object).await?) as Result<_, warp::Rejection>
            } else {
                Err(warp::reject::not_found())
            }
        })
}
