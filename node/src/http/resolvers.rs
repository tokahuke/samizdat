//! Bridges from the Samizdat world to the HTTP world.

use futures::stream;
use http::Response;
use hyper::Body;
use rocksdb::WriteBatch;
use std::convert::TryInto;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::models::{IdentityRef, ItemPath, Locator, ObjectRef, SeriesRef};

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
    log::info!("Resolving {object:?}");

    let iter = if let Some(iter) = object.iter_skip_header()? {
        log::info!("Found local hash {}", object.hash());
        Some(iter)
    } else {
        log::info!("Hash {} not found locally. Querying hubs", object.hash());
        hubs().query(*object.hash(), QueryKind::Object).await;
        object.iter_skip_header()?
    };

    // Respond with found or not found.
    if let Some((metadata, iter)) = object.metadata()?.zip(iter) {
        object.touch()?;
        let resolved = Resolved {
            content_type: metadata.header.content_type().to_owned(),
            content_size: metadata.content_size,
            ext_headers: ext_headers
                .into_iter()
                .chain([
                    ("X-Samizdat-Bookmark", object.is_bookmarked()?.to_string()),
                    (
                        "X-Samizdat-Is-Draft",
                        metadata.header.is_draft().to_string(),
                    ),
                    ("X-Samizdat-Object", object.hash().to_string()),
                ])
                .collect(),
            body: Body::wrap_stream(stream::iter(
                crate::utils::chunks(1000, iter).map(|thing| thing.map_err(|err| err.to_string())),
            )),
        };

        Ok(resolved.try_into())
    } else {
        let not_resolved = NotResolved {
            message: format!("Object {} not found", object.hash()),
        };

        Ok(not_resolved.try_into())
    }
}

/// Tries to find an object as a collection item, asking the Samizdat network if
/// necessary.
pub async fn resolve_item(
    locator: Locator<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving item {locator}");

    let maybe_item = if let Some(item) = locator.get()? {
        log::info!("Found item {locator} locally. Resolving object.");
        Some(item)
    } else {
        log::info!("Item not found locally. Querying hubs.");
        hubs().query(locator.hash(), QueryKind::Item).await;

        locator.get()?
    };

    if let Some(item) = maybe_item {
        resolve_object(
            item.object()?,
            ext_headers.into_iter().chain([(
                "X-Samizdat-Collection",
                locator.collection().hash().to_string(),
            )]),
        )
        .await
    } else {
        let not_resolved = NotResolved {
            message: format!("Item {locator} not found"),
        };

        Ok(not_resolved.try_into())
    }
}

/// Tries to find an object as an item the collection corresponding to the latest
/// version of a series, asking the Samizdat network if necessary.
pub async fn resolve_series(
    series: SeriesRef,
    name: ItemPath<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving series item {series}/{name}");

    log::info!("Ensuring series {series} is fresh");
    if !series.is_fresh()? {
        log::info!("Series is not fresh. Asking the network...");
        if let Some(latest) = hubs().get_latest(&series).await {
            log::info!("Found an edition (new or existing): {latest:?}. Inserting");
            series.advance(&latest)?;
            series.refresh()?;
        } else {
            log::info!("No edition returned from the network for series {series}. Does it exist?");
            series.mark_delayed()?;
        }
    }

    log::info!("Trying to find path in each edition");
    let mut empty = true;

    for edition in series.get_editions()? {
        empty = false;
        log::info!("Trying collection {:?}", edition.collection());
        let locator = edition.collection().locator_for(name.clone());

        let maybe_item = if let Some(item) = locator.get()? {
            log::info!("Found item {locator} locally. Resolving object.");
            Some(item)
        } else {
            log::info!("Item not found locally. Querying hubs.");
            hubs().query(locator.hash(), QueryKind::Item).await;

            locator.get()?
        };

        if let Some(item) = maybe_item {
            return resolve_object(
                item.object()?,
                ext_headers.into_iter().chain([
                    (
                        "X-Samizdat-Collection",
                        locator.collection().hash().to_string(),
                    ),
                    ("X-Samizdat-Series", series.public_key().to_string()),
                ]),
            )
            .await;
        }
    }

    if empty {
        log::info!("No local editions found for series {series}");
    }

    let not_resolved = NotResolved {
        message: format!("Item {series}/{name} not found"),
    };

    Ok(not_resolved.try_into())
}

pub async fn resolve_identity(
    identity_ref: IdentityRef,
    name: ItemPath<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving identity {identity_ref}/{name}");

    let identity = if let Some(identity) = identity_ref.get()? {
        log::info!("Fond identity {identity_ref} locally. Resolving series.");
        identity
    } else {
        log::info!("Identity not found locally. Querying hubs.");
        if let Some(identity) = hubs().get_identity(&identity_ref).await {
            let mut batch = WriteBatch::default();
            identity.insert(&mut batch);
            crate::db().write(batch)?;

            identity
        } else {
            let not_resolved = NotResolved {
                message: format!("Identity {identity_ref} not found"),
            };

            return Ok(not_resolved.try_into());
        }
    };

    resolve_series(identity.series(), name, ext_headers).await
}
