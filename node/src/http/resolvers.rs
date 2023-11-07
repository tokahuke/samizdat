//! Bridges from the Samizdat world to the HTTP world.

use futures::TryStreamExt;
use http::Response;
use hyper::Body;
use rocksdb::WriteBatch;
use std::convert::TryInto;

use samizdat_common::rpc::QueryKind;

use crate::hubs;
use crate::models::{IdentityRef, ItemPath, Locator, ObjectRef, SeriesRef};
use crate::system::{ReceivedItem, ReceivedObject};

/// Am HTTP response for an object that has been resolved.
pub struct Resolved {
    /// The body to be sent to the client, streaming the object's content.
    body: Body,
    /// The cotent type of this object.
    content_type: String,
    /// Extra headers to be sent in the HTTP response.
    ext_headers: Vec<(&'static str, String)>,
}

impl TryInto<Response<Body>> for Resolved {
    type Error = http::Error;
    fn try_into(self) -> Result<Response<Body>, http::Error> {
        let mut builder = http::Response::builder().header("Content-Type", self.content_type);

        for (header, value) in self.ext_headers {
            builder = builder.header(header, value);
        }

        builder.status(http::StatusCode::OK).body(self.body)
    }
}

/// An HTTP response for an object that has *not* been resolved.
pub struct NotResolved {
    /// The message to be relayed to the client.
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

/// Creates the HTTP response for an object that has been found outside the node and is
/// being currently downloaded.
async fn resolve_new_object(
    received_object: ReceivedObject,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Resolved, crate::Error> {
    let object = received_object.object_ref();
    let header = received_object.metadata().header.clone();
    let content_stream = received_object.into_content_stream();

    Ok(Resolved {
        content_type: header.content_type().to_owned(),
        ext_headers: ext_headers
            .into_iter()
            .chain([
                ("ETag", format!("\"{}\"", object.hash())),
                // New objects are never bookmarked
                ("X-Samizdat-Bookmark", "false".to_owned()),
                // ("X-Samizdat-Is-Draft", header.is_draft().to_string()),
                ("X-Samizdat-Object", object.hash().to_string()),
            ])
            .collect(),
        body: Body::wrap_stream(content_stream.map_err(|err| err.to_string())),
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
            ])
            .collect(),
        body: Body::wrap_stream(content_stream.map_err(|err| err.to_string())),
    })
}

/// Tries to find an object, asking the Samizdat network if necessary.
pub async fn resolve_object(
    object: ObjectRef,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
    log::info!("Resolving {object:?}");
    if object.exists()? {
        log::info!("Found local hash {}", object.hash());
        return Ok(resolve_existing_object(object, ext_headers)?.try_into());
    }

    log::info!("Hash {} not found locally. Querying hubs", object.hash());
    match hubs().query(*object.hash(), QueryKind::Object).await {
        // This should not be possible!!
        Some(ReceivedItem::ExistingObject(object)) => {
            log::warn!(
                "After querying hubs, found local hash {}. This should be impossible!",
                object.hash()
            );
            Ok(resolve_existing_object(object, ext_headers)?.try_into())
        }
        Some(ReceivedItem::NewObject(received_object)) => {
            Ok(resolve_new_object(received_object, ext_headers)
                .await?
                .try_into())
        }
        None => {
            let not_resolved = NotResolved {
                message: format!("Object {} not found", object.hash()),
            };

            Ok(not_resolved.try_into())
        }
    }
}

/// Tries to find an object as a collection item, asking the Samizdat network if
/// necessary.
pub async fn resolve_item(
    locator: Locator<'_>,
    ext_headers: impl IntoIterator<Item = (&'static str, String)>,
) -> Result<Result<Response<Body>, http::Error>, crate::Error> {
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
        return resolve_object(object, ext_headers).await;
    }

    log::info!("Item {locator} not found locally. Querying hubs");
    match hubs().query(locator.hash(), QueryKind::Object).await {
        // This should not be possible!!
        Some(ReceivedItem::ExistingObject(object)) => {
            log::warn!(
                "After querying hubs, found local hash {} for item {locator}",
                object.hash()
            );
            Ok(resolve_existing_object(object, ext_headers)?.try_into())
        }
        Some(ReceivedItem::NewObject(received_object)) => {
            log::warn!(
                "After querying hubs, found new hash {} for item {locator}",
                received_object.object_ref().hash()
            );
            Ok(resolve_new_object(received_object, ext_headers)
                .await?
                .try_into())
        }
        None => {
            let not_resolved = NotResolved {
                message: format!("Item {locator} not found"),
            };

            Ok(not_resolved.try_into())
        }
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

    // Was a `for`. Now, we look only at the top edition.
    if let Some(edition) = series.get_editions().next() {
        let edition = edition?;
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

/// Tries to find an item in a collection, accessed by an identity handle, asking the
/// Samizdat network if necessary.
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
