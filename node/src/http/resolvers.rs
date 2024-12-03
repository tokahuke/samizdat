//! Bridges from the Samizdat world to the HTTP world.

use axum::body::Body;
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use tokio::time::Instant;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::identity_dapp::identity_provider;
use crate::models::{EditionKind, ItemPath, Locator, ObjectRef, SeriesRef};
use crate::system::{ReceivedItem, ReceivedObject};

/// Am HTTP response for an object that has been resolved.
pub struct Resolved {
    /// The body to be sent to the client, streaming the object's content.
    body: Body,
    /// The content type of this object.
    content_type: String,
    /// Extra headers to be sent in the HTTP response.
    ext_headers: Vec<(&'static str, String)>,
}

impl IntoResponse for Resolved {
    fn into_response(self) -> Response {
        let mut builder = http::Response::builder().header("Content-Type", self.content_type);

        for (header, value) in self.ext_headers {
            builder = builder.header(header, value);
        }

        builder
            .status(http::StatusCode::OK)
            .body(self.body)
            .expect("can build resolved object response")
    }
}

/// An HTTP response for an object that has *not* been resolved.
pub struct NotResolved {
    /// The message to be relayed to the client.
    message: String,
}

impl IntoResponse for NotResolved {
    fn into_response(self) -> Response {
        http::Response::builder()
            .header("Content-Type", "text/plain")
            .status(http::StatusCode::NOT_FOUND)
            .body(self.message.into())
            .expect("can build not resolved object response")
    }
}

/// Creates the HTTP response for an object that has been found outside the node and is
/// being currently downloaded.
async fn resolve_new_object(
    received_object: ReceivedObject,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Resolved, crate::Error> {
    let object = received_object.object_ref();
    let header = received_object.metadata().header.clone();
    let query_duration = received_object.query_duration();
    let content_stream = received_object.into_content_stream();

    Ok(Resolved {
        content_type: header.content_type().to_owned(),
        ext_headers: ext_headers
            .into_iter()
            .chain([
                ("ETag", format!("\"{}\"", object.hash())),
                // New objects are never bookmarked
                ("X-Samizdat-Bookmark", "false".to_owned()),
                ("X-Samizdat-Is-Draft", header.is_draft().to_string()),
                ("X-Samizdat-Object", object.hash().to_string()),
                (
                    "X-Samizdat-Query-Duration",
                    query_duration.as_secs_f64().to_string(),
                ),
            ])
            .collect(),
        body: Body::from_stream(content_stream.map_err(|err| err.to_string())),
    })
}

/// Creates the HTTP response for an object that has been found in the local database and
/// does not need to be downloaded.
fn resolve_existing_object(
    object: ObjectRef,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Resolved, crate::Error> {
    let metadata = object.metadata()?.expect("object exists");
    let content_stream = object.stream_content(true)?.expect("object exists");

    Ok(Resolved {
        content_type: metadata.header.content_type().to_owned(),
        ext_headers: ext_headers
            .into_iter()
            .chain([
                ("ETag", format!("\"{}\"", object.hash())),
                ("X-Samizdat-Bookmark", object.is_bookmarked()?.to_string()),
                (
                    "X-Samizdat-Is-Draft",
                    metadata.header.is_draft().to_string(),
                ),
                ("X-Samizdat-Object", object.hash().to_string()),
                ("X-Samizdat-Query-Duration", "0".to_owned()),
            ])
            .collect(),
        body: Body::from_stream(content_stream.map_err(|err| err.to_string())),
    })
}

/// Tries to find an object, asking the Samizdat network if necessary.
pub async fn resolve_object(
    object: ObjectRef,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
    deadline: Instant,
) -> Result<Response, crate::Error> {
    log::info!("Resolving {object:?}");

    if object.is_null() {
        return Ok(NotResolved {
            message: format!("Object {} not found", object.hash()),
        }
        .into_response());
    }

    if object.exists()? {
        log::info!("Found local hash {}", object.hash());
        return Ok(resolve_existing_object(object, ext_headers)?.into_response());
    }

    log::info!("Hash {} not found locally. Querying hubs", object.hash());
    match hubs()
        .query(*object.hash(), QueryKind::Object, deadline)
        .await
    {
        // This should not be possible!!
        Some(ReceivedItem::ExistingObject(object)) => {
            log::warn!(
                "After querying hubs, found local hash {}. This should be impossible!",
                object.hash()
            );
            Ok(resolve_existing_object(object, ext_headers)?.into_response())
        }
        Some(ReceivedItem::NewObject(received_object)) => {
            Ok(resolve_new_object(received_object, ext_headers)
                .await?
                .into_response())
        }
        None => {
            let not_resolved = NotResolved {
                message: format!("Object {} not found", object.hash()),
            };

            Ok(not_resolved.into_response())
        }
    }
}

/// Tries to find an object as a collection item, asking the Samizdat network if
/// necessary.
pub async fn resolve_item(
    locator: Locator<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
    deadline: Instant,
) -> Result<Response, crate::Error> {
    // Add extra headers for item:
    let ext_headers = ext_headers.into_iter().chain([(
        "X-Samizdat-Collection",
        locator.collection().hash().to_string(),
    )]);

    log::info!("Resolving item {locator}");
    if let Some(item) = locator.get()? {
        // If the object is known locally, we can simply deffer to querying the object.
        log::info!("Found item {locator} locally. Resolving object.");
        let object = item.object().expect("found invalid object for item");
        return resolve_object(object, ext_headers, deadline).await;
    }

    log::info!("Item {locator} not found locally. Querying hubs");
    match hubs()
        .query(locator.hash(), QueryKind::Item, deadline)
        .await
    {
        Some(ReceivedItem::ExistingObject(object)) if object.is_null() => Ok(NotResolved {
            message: format!("Item {locator} not found"),
        }
        .into_response()),
        Some(ReceivedItem::ExistingObject(object)) => {
            log::warn!(
                "After querying hubs, found local hash {} for item {locator}",
                object.hash()
            );
            Ok(resolve_existing_object(object, ext_headers)?.into_response())
        }
        Some(ReceivedItem::NewObject(received_object)) => {
            log::warn!(
                "After querying hubs, found new hash {} for item {locator}",
                received_object.object_ref().hash()
            );
            Ok(resolve_new_object(received_object, ext_headers)
                .await?
                .into_response())
        }
        None => {
            let not_resolved = NotResolved {
                message: format!("Item {locator} not found"),
            };

            Ok(not_resolved.into_response())
        }
    }
}

/// Tries to find an object as an item the collection corresponding to the latest
/// version of a series, asking the Samizdat network if necessary.
pub async fn resolve_series(
    series: SeriesRef,
    name: ItemPath<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
    deadline: Instant,
) -> Result<Response<Body>, crate::Error> {
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

    log::info!("Trying to find path in freshest edition");
    let mut empty = true;

    for edition in series.get_editions() {
        let edition = edition?;
        empty = false;
        log::info!("Trying collection {:?}", edition.collection());
        let locator = edition.collection().locator_for(name.clone());

        let maybe_item = if let Some(item) = locator.get()? {
            log::info!("Found item {locator} locally. Resolving object.");
            Some(item)
        } else {
            log::info!("Item not found locally. Querying hubs.");
            hubs()
                .query(locator.hash(), QueryKind::Item, deadline)
                .await;

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
                    ("X-Samizdat-Edition", edition.timestamp().to_string()),
                ]),
                deadline,
            )
            .await;
        }

        if edition.kind() == EditionKind::Base {
            break;
        }
    }

    if empty {
        log::info!("No local editions found for series {series}");
    }

    let not_resolved = NotResolved {
        message: format!("Item {series}/{name} not found"),
    };

    Ok(not_resolved.into_response())
}

/// Tries to find an item in a collection, accessed by an identity handle, asking the
/// Samizdat network if necessary.
pub async fn resolve_identity(
    identity: &str,
    name: ItemPath<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
    deadline: Instant,
) -> Result<Response<Body>, crate::Error> {
    log::info!("Resolving identity {identity}/{name}");
    let Some(identity) = identity_provider().get_cached(identity).await? else {
        let not_resolved = NotResolved {
            message: format!("Identity {identity} not found"),
        };

        return Ok(not_resolved.into_response());
    };

    resolve_series(identity.series()?, name, ext_headers, deadline).await
}
