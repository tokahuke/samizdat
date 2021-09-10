//! Bridges from the Samizdat world to the HTTP world.

use futures::stream;
use http::Response;
use hyper::Body;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::models::{Locator, ObjectRef, SeriesRef};

pub async fn resolve_object(
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

pub async fn resolve_item(
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

pub async fn resolve_series(
    series: SeriesRef,
    name: &str,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    if let Some(latest) = series.get_latest_fresh()? {
        log::info!("Have a fresh result locally. Will resolve this item.");
        let locator = latest.collection().locator_for(name);
        Ok(resolve_item(locator).await?)
    } else if series.is_locally_owned()? {
        log::info!("Does not have a fresh result, but is owned. So, a result doesn't exist.");
        Ok(http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(format!("series {} is empty", series))))
    } else if let Some(latest) = hubs().get_latest(&series).await {
        log::info!("Does not have a fresh result, but is not owned locally. Query the network!");
        series.advance(&latest)?;
        let locator = latest.collection().locator_for(name);
        Ok(resolve_item(locator).await?)
    } else {
        log::info!("Not found!");
        Ok(http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(format!("series {} not found", series))))
    }
}