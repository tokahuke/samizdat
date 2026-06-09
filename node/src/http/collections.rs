//! Collections API.

use axum::{Json, Router, extract::DefaultBodyLimit, routing::post};
use futures::FutureExt;
use serde_derive::Deserialize;

use crate::{
    access::AccessRight,
    http::ApiResponse,
    models::{CollectionRef, ItemPathBuf, ObjectRef},
    security_scope,
};

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

    Router::new().route(
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
