//! Collections API.

use axum::extract::{DefaultBodyLimit, Path};
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::FutureExt;
use serde_derive::Deserialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;
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

    #[serde_as]
    #[derive(Deserialize)]
    struct ItemPath {
        #[serde_as(as = "DisplayFromStr")]
        hash: Hash,
        #[serde(default)]
        name: String,
    }

    #[serde_as]
    #[derive(Deserialize)]
    struct CollectionPath {
        #[serde_as(as = "DisplayFromStr")]
        hash: Hash,
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
            .layer(
                tower::ServiceBuilder::new()
                    .layer(security_scope!(AccessRight::ManageCollections))
                    .layer(DefaultBodyLimit::disable()),
            ),
        )
        .route(
            // Gets the contents of a collection item.
            "/:hash/*name",
            get(
                |Path(ItemPath { hash, name }): Path<ItemPath>,
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
        .route(
            // Gets the contents of a collection item.
            "/:hash/",
            get(
                |Path(CollectionPath { hash }): Path<CollectionPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        let collection = CollectionRef::new(hash);
                        let path = "".into();
                        let locator = collection.locator_for(path);

                        resolve_item(locator, [], Instant::now() + timeout).await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            "/:hash",
            get(
                |Path(CollectionPath { hash }): Path<CollectionPath>| async move {
                    Redirect::permanent(&format!("{hash}/"))
                },
            ),
        )
}
