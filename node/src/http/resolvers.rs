//! Bridges from the Samizdat world to the HTTP world.

use futures::stream;
use http::Response;
use hyper::Body;
use std::convert::TryInto;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::models::{Locator, ObjectRef, SeriesRef};

pub struct Resolved {
    body: Body,
    content_type: String,
    content_size: usize,
    ext_headers: Vec<(&'static str, String)>,
}

impl TryInto<Response<Body>> for Resolved {
    type Error = http::Error;
    fn try_into(self) -> Result<Response<Body>, http::Error> {
        let mut builder = http::Response::builder()
            .header("Content-Type", self.content_type)
            .header("Content-Size", self.content_size);

        for (header, value) in self.ext_headers {
            builder = builder.header(header, value);
        }

        builder
            .status(http::StatusCode::OK)
            // TODO: Bleh! Tidy-up this mess!
            .body(self.body)
    }
}

pub struct NotResolved {
    message: String,
}

impl TryInto<Response<Body>> for NotResolved {
    type Error = http::Error;
    fn try_into(self) -> Result<Response<Body>, http::Error> {
        http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(Body::from(self.message))
    }
}

/// Tries to find an object, asking the Samizdat network if necessary.
pub async fn resolve_object(
    object: ObjectRef,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving {:?}", object);

    let stream = if let Some(stream) = object.iter()? {
        log::info!("found local hash {}", object.hash());
        Some(stream)
    } else {
        log::info!("hash {} not found locally. Querying hubs", object.hash());
        hubs().query(*object.hash(), QueryKind::Object).await;
        object.iter()?
    };

    // Respond with found or not found.
    if let Some((metadata, iter)) = object.metadata()?.zip(stream) {
        object.touch()?;
        let resolved = Resolved {
            content_type: metadata.content_type,
            content_size: metadata.content_size,
            ext_headers: ext_headers
                .into_iter()
                .chain(vec![(
                    "X-Samizdat-Bookmark",
                    object.is_bookmarked()?.to_string(),
                )])
                .collect(),
            body: Body::wrap_stream(stream::iter(
                iter.into_iter()
                    .map(|thing| thing.map_err(|err| err.to_string())),
            )),
        };

        Ok(resolved.try_into())
    } else {
        let not_resolved = NotResolved {
            message: format!("object {} not found", object.hash()),
        };

        Ok(not_resolved.try_into())
    }
}

/// Tries to find an object as a collection item, asking the Samizdat network if
/// necessary.
pub async fn resolve_item(
    locator: Locator<'_>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving item {}", locator);

    let maybe_item = if let Some(item) = locator.get()? {
        log::info!("found item {} locally. Resolving object.", locator);
        Some(item)
    } else {
        log::info!("item not found locally. Querying hubs.");
        hubs().query(locator.hash(), QueryKind::Item).await;

        locator.get()?
    };

    if let Some(item) = maybe_item {
        resolve_object(item.object()?, vec![]).await
    } else {
        let not_resolved = NotResolved {
            message: format!("item {} not found", locator),
        };

        Ok(not_resolved.try_into())
    }
}

/// Tries to find an object as an item the collection correspoinding to the latest
/// version of a series, asking the Samizdat network if necessary.
pub async fn resolve_series(
    series: SeriesRef,
    name: &str,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving series {}/{}", series, name);
    if let Some(latest) = series.get_latest_fresh()? {
        log::info!("Have a fresh result locally. Will resolve this item.");
        let locator = latest.collection().locator_for(name);
        Ok(resolve_item(locator).await?)
    } else if series.is_locally_owned()? {
        log::info!("Does not have a fresh result, but is owned. So, a result doesn't exist.");
        let not_resolved = NotResolved { 
            message: format!("series {} is empty", series),
        };
        Ok(not_resolved.try_into())
    } else if let Some(latest) = hubs().get_latest(&series).await {
        log::info!("Does not have a fresh result, but is not owned locally. Query the network!");
        series.advance(&latest)?;
        let locator = latest.collection().locator_for(name);
        Ok(resolve_item(locator).await?)
    } else if let Some(mut latest) = series.get_latest()? {
        log::info!("The not fresh result will have to do... Will resolve this item.");
        // Refresh:
        latest.make_fresh();
        series.advance(&latest)?;
        let locator = latest.collection().locator_for(name);
        Ok(resolve_item(locator).await?)
    } else {
        log::info!("Not found!");
        let not_resolved = NotResolved {
            message: format!("series {} not found", series),
        };
        Ok(not_resolved.try_into())
    }
}
