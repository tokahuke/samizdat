//! Objects API.

use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query},
    routing::{delete, get, post},
};
use futures::FutureExt;
use samizdat_common::{
    Hash,
    db::{Droppable, readonly_tx, writable_tx},
};
use serde_derive::Deserialize;
use serde_with::{DisplayFromStr, serde_as};

use crate::{
    access::AccessRight,
    http::ContentType,
    models::{BookmarkType, ObjectHeader, ObjectRef},
    security_scope,
};

use super::ApiResponse;

/// The entrypoint of the object API.
pub fn api() -> Router {
    Router::new()
        .merge(object())
        .merge(object_bookmark())
        .merge(object_stats())
}

/// Manages the `_objects` route.
fn object() -> Router {
    #[serde_as]
    #[derive(Deserialize)]
    struct ObjectPath {
        #[serde_as(as = "DisplayFromStr")]
        hash: Hash,
    }

    #[derive(Deserialize)]
    #[serde(rename = "kebab-case")]
    struct PostObjectQuery {
        #[serde(default)]
        bookmark: bool,
        #[serde(default)]
        is_draft: bool,
    }

    #[derive(Deserialize)]
    #[serde(rename = "kebab-case")]
    struct PostReissueQuery {
        #[serde(default)]
        bookmark: bool,
    }

    Router::new()
        .route(
            "/",
            post(
                |ContentType(content_type): ContentType,
                 Query(query): Query<PostObjectQuery>,
                 bytes: Bytes| {
                    async move {
                        let header = ObjectHeader::new(content_type, query.is_draft)?;
                        let object = tokio::task::spawn_blocking(move || {
                            ObjectRef::build(
                                header,
                                query.bookmark,
                                bytes.into_iter().map(Result::Ok),
                            )
                        })
                        .await
                        .expect("Object build task failed")?;
                        Ok(object.hash().to_string())
                    }
                    .map(ApiResponse)
                },
            )
            .layer(
                tower::ServiceBuilder::new()
                    .layer(security_scope!(AccessRight::ManageObjects))
                    .layer(DefaultBodyLimit::disable()),
            ),
        )
        .route(
            "/{hash}",
            delete(|Path(ObjectPath { hash }): Path<ObjectPath>| {
                async move { ObjectRef::new(hash).drop_if_exists() }.map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageObjects)),
        )
        .route(
            "/{hash}/reissue",
            post(
                |Path(ObjectPath { hash }): Path<ObjectPath>,
                 Query(query): Query<PostReissueQuery>| {
                    async move {
                        tokio::task::spawn_blocking(move || {
                            ObjectRef::new(hash)
                                .reissue(query.bookmark)
                                .map(|reissued| {
                                    reissued.map(|reissued| reissued.hash().to_string())
                                })
                        })
                        .await
                        .expect("Object reissue task panicked")
                    }
                    .map(ApiResponse)
                },
            )
            .layer(security_scope!(AccessRight::ManageObjects)),
        )
}

fn object_bookmark() -> Router {
    Router::new()
        .route(
            // Bookmarks an object. This will prevent the object from being automatically removed
            // by the vacuum daemon.
            "/{hash}/bookmark",
            post(|Path(hash): Path<Hash>| {
                async move {
                    writable_tx(|tx| {
                        ObjectRef::new(hash).bookmark(BookmarkType::User).mark(tx)?;
                        Ok(())
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageBookmarks)),
        )
        .route(
            // Returns whether an object is bookmarked or not.
            //
            // # Warning
            //
            // By now, this returns `200 OK` even if the object does not exist.
            "/{hash}/bookmark",
            get(|Path(hash): Path<Hash>| {
                async move {
                    readonly_tx(|tx| {
                        ObjectRef::new(hash)
                            .bookmark(BookmarkType::User)
                            .is_marked(tx)
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::ManageBookmarks)),
        )
        .route(
            // Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
            "/{hash}/bookmark",
            delete(|Path(hash): Path<Hash>| {
                async move {
                    writable_tx(|tx| {
                        ObjectRef::new(hash)
                            .bookmark(BookmarkType::User)
                            .unmark(tx)?;
                        Ok(())
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageBookmarks)),
        )
}

fn object_stats() -> Router {
    Router::new()
        .route(
            // Returns the internal reference count on the object.
            //
            // # Warning
            //
            // By now, this returns `200 OK` even if the object does not exist.
            "/{hash}/reference-count",
            get(|Path(hash): Path<Hash>| {
                async move {
                    readonly_tx(|tx| {
                        ObjectRef::new(hash)
                            .bookmark(BookmarkType::Reference)
                            .get_count(tx)
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::GetObjectStats)),
        )
        .route(
            "/{hash}/stats",
            get(|Path(hash): Path<Hash>| {
                async move { readonly_tx(|tx| ObjectRef::new(hash).statistics(tx)) }
                    .map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::GetObjectStats)),
        )
        .route(
            "/{hash}/stats/byte-usefulness",
            get(|Path(hash): Path<Hash>| {
                async move {
                    readonly_tx(|tx| {
                        ObjectRef::new(hash).statistics(tx).map(|stats| {
                            stats.map(|stats| {
                                stats.byte_usefulness(&crate::models::UsePrior::default())
                            })
                        })
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::GetObjectStats)),
        )
}
