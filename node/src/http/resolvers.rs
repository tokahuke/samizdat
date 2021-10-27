//! Bridges from the Samizdat world to the HTTP world.

use futures::stream;
use http::Response;
use hyper::Body;
use std::convert::TryInto;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::models::{ItemPath, Locator, ObjectRef, SeriesRef};

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

    let iter = if let Some(iter) = object.iter_skip_header()? {
        log::info!("found local hash {}", object.hash());
        Some(iter)
    } else {
        log::info!("hash {} not found locally. Querying hubs", object.hash());
        hubs().query(*object.hash(), QueryKind::Object).await;
        object.iter_skip_header()?
    };

    // Respond with found or not found.
    if let Some((metadata, iter)) = object.metadata()?.zip(iter) {
        object.touch()?;
        let resolved = Resolved {
            content_type: metadata.header.content_type,
            content_size: metadata.content_size,
            ext_headers: ext_headers
                .into_iter()
                .chain(vec![
                    ("X-Samizdat-Bookmark", object.is_bookmarked()?.to_string()),
                    ("X-Samizdat-Is-Draft", metadata.header.is_draft.to_string()),
                    (
                        "X-Samizdat-Created-At",
                        metadata.header.created_at.to_string(),
                    ),
                ])
                .collect(),
            body: Body::wrap_stream(stream::iter(
                crate::utils::chunks(1000, iter).map(|thing| thing.map_err(|err| err.to_string())),
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
    name: ItemPath<'_>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving series {}/{}", series, name.as_str());

    log::info!("Ensuring series is fresh");
    if !series.is_fresh()? {
        log::info!("Series is not fresh. Ask the network");
        if let Some(latest) = hubs().get_latest(&series).await {
            log::info!("Found a series items (new or existing). Inserting");
            series.advance(&latest)?;
        }

        log::info!("Seting series as fresh");
        series.refresh()?;
    }

    log::info!("Trying to find path in in each series item");
    for item in series.get_items()? {
        let locator = item.collection().locator_for(name.clone());

        let maybe_item = if let Some(item) = locator.get()? {
            log::info!("found item {} locally. Resolving object.", locator);
            Some(item)
        } else {
            log::info!("item not found locally. Querying hubs.");
            hubs().query(locator.hash(), QueryKind::Item).await;

            locator.get()?
        };

        if let Some(item) = maybe_item {
            return resolve_object(item.object()?, vec![]).await;
        }
    }

    let not_resolved = NotResolved {
        message: format!("item {}/{} not found", series, name.as_str()),
    };

    Ok(not_resolved.try_into())
}
