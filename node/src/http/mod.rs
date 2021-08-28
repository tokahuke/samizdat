mod returnable;

pub use returnable::{Return, Returnable};

use futures::stream;
use http::Response;
use hyper::Body;
use std::time::Duration;
use warp::path::Tail;
use warp::Filter;
use serde_derive::{Deserialize};

use samizdat_common::rpc::QueryKind;
use samizdat_common::{Hash, Key};

use crate::cache::{CollectionRef, Locator, ObjectRef, SeriesOwner, SeriesRef};
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
        log::info!("hash {} not found locally. Querying hubs", object.hash);
        hubs().query(object.hash, QueryKind::Object).await;
        object.iter()?
    };

    // Respond with found or not found.
    if let Some((metadata, iter)) = object.metadata()?.zip(stream) {
        let response = http::Response::builder()
            .header("Content-Type", metadata.content_type)
            .header("Content-Size", metadata.content_size)
            .status(http::StatusCode::OK)
            // TODO: Bleh! Tidy-up this mess!
            .body(Body::wrap_stream(stream::iter(
                iter.into_iter()
                    .map(|thing| thing.map_err(|err| err.to_string())),
            )));

        Ok(response)
    } else {
        let response = http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(format!("object {} not found", object.hash)));

        Ok(response)
    }
}

async fn resolve_item(
    locator: Locator<'_>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    let maybe_item = if let Some(item) = locator.get()? {
        log::info!("found item {} locally. Resolving object.", locator);
        Some(item)
    } else {
        log::info!("item not found locally. Querying hubs.");
        hubs().query(locator.hash(), QueryKind::Item).await;
        locator.get()?
    };

    if let Some(item) = maybe_item {
        resolve_object(item.object()?).await
    } else {
        let response = http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(format!("item {} not found", locator)));

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
            let locator = collection.locator_for(name.as_str());
            Ok(resolve_item(locator).await?) as Result<_, warp::Rejection>
        })
}

pub fn post_series_owner(
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::post())
        .map(|series_owner_name: String| {
            let series_owner = SeriesOwner::create(&series_owner_name, Duration::from_secs(3_600))?;
            Ok(series_owner.series().to_string())
        })
        .map(reply)
}

pub fn get_series_owner() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone
{
    warp::path!("_seriesowners" / String)
        .and(warp::get())
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            Ok(maybe_owner.map(|owner| owner.series().to_string()))
        })
        .map(reply)
}

pub fn get_series_owners() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners")
        .and(warp::get())
        .map(|| {
            let series = SeriesOwner::get_all()?;
            Ok(returnable::Json(series))
        })
        .map(reply)
}

pub fn post_series() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        #[serde(default)]
        #[serde(with = "humantime_serde")]
        ttl: Option<std::time::Duration>,
    }

    warp::path!("_seriesowners" / String / "collections" / Hash)
        .and(warp::post())
        .and(warp::query())
        .map(|series_owner_name: String, collection, request: Request| {
            if let Some(series_owner) = SeriesOwner::get(&series_owner_name)? {
                let series = series_owner.advance(CollectionRef::new(collection), request.ttl)?;
                Ok(Some(returnable::Json(series)))
            } else {
                Ok(None)
            }
        })
        .map(reply)
}

pub fn get_item_by_series() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_series" / Key / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|series_key: Key, name: Tail| async move {
            let series = SeriesRef::new(series_key);

            if let Some(latest) = series.get_latest_fresh()? {
                log::info!("Have a fresh result locally. Will resolve this item.");
                let locator = latest.collection().locator_for(name.as_str());
                Ok(resolve_item(locator).await?) as Result<_, warp::Rejection>
            } else if series.is_locally_owned()? {
                log::info!("Does not have a fresh result, but is owned. So, a result doesn't exist.");
                Ok(http::Response::builder()
                    .header("Content-Type", "text/plain")
                    .status(http::StatusCode::NOT_FOUND)
                    .body(Body::from(format!("series {} is empty", series))))
                    as Result<_, warp::Rejection>
            } else if let Some(latest) = hubs().get_latest(&series).await {
                log::info!("Does not have a fresh result, but is not owned locally. Query the network!");
                series.advance(&latest)?;
                let locator = latest.collection().locator_for(name.as_str());
                Ok(resolve_item(locator).await?) as Result<_, warp::Rejection>
            } else {
                log::info!("Not found!");
                Ok(http::Response::builder()
                    .header("Content-Type", "text/plain")
                    .status(http::StatusCode::NOT_FOUND)
                    .body(Body::from(format!("series {} not found", series))))
                    as Result<_, warp::Rejection>
            }
        })
}
