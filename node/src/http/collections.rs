//! Collections API.

use axum::extract::Path;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::FutureExt;
use serde_derive::Deserialize;
use tokio::time::Instant;

use samizdat_common::Hash;

use crate::access::AccessRight;
use crate::http::{ApiResponse, PageResponse, SamizdatTimeout};
use crate::models::{CollectionRef, ItemPathBuf, ObjectRef};
use crate::security_scope;

use super::resolvers::resolve_item;

/// The entrypoint of the collection public API.
pub fn api() -> Router {
    Router::new().merge(collection())
}

fn collection() -> Router {
    #[derive(Deserialize)]
    struct PostCollectionRequest {
        #[serde(default)]
        is_draft: bool,
        hashes: Vec<(String, String)>,
    }

    #[derive(Deserialize)]
    struct GetItemPath {
        hash: Hash,
        name: String,
    }

    Router::new()
        .route(
            // Uploads a new collection.
            "/",
            post(|Json(request): Json<PostCollectionRequest>| {
                async move {
                    let collection = tokio::task::spawn_blocking(move || {
                        CollectionRef::build(
                            request.is_draft,
                            request
                                .hashes
                                .into_iter()
                                .map(|(name, hash)| {
                                    Ok((ItemPathBuf::from(name), ObjectRef::new(hash.parse()?)))
                                })
                                .collect::<Result<Vec<_>, crate::Error>>()?,
                        )
                    })
                    .await
                    .expect("Collection build task panicked")?;
                    Ok(collection.hash().to_string())
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageCollections)),
        )
        .route(
            // Gets the contents of a collection item.
            "/:hash/*path",
            get(
                |Path(GetItemPath { hash, name }): Path<GetItemPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        let collection = CollectionRef::new(hash);
                        let path = name.as_str().into();
                        let locator = collection.locator_for(path);

                        resolve_item(locator, [], Instant::now() + timeout).await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
}
