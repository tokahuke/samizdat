//! Collections API.

use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use axum::{Json, Router};
use futures::FutureExt;
use serde_derive::Deserialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;

use samizdat_common::Hash;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::{CollectionRef, ItemPathBuf, ObjectRef};
use crate::security_scope;

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
}
