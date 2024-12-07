//! Objects API.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query};
use axum::routing::{delete, get, post};
use axum::Router;
use futures::FutureExt;
use samizdat_common::Hash;
use serde_derive::Deserialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use tokio::time::Instant;

use crate::access::AccessRight;
use crate::http::ContentType;
use crate::models::{BookmarkType, Droppable, ObjectHeader, ObjectRef};
use crate::security_scope;

use super::resolvers::resolve_object;
use super::{ApiResponse, PageResponse, SamizdatTimeout};

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
            "/:hash",
            get(
                |Path(ObjectPath { hash }): Path<ObjectPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        Ok(
                            resolve_object(ObjectRef::new(hash), vec![], Instant::now() + timeout)
                                .await?,
                        )
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
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
            "/:hash",
            delete(|Path(ObjectPath { hash }): Path<ObjectPath>| {
                async move { ObjectRef::new(hash).drop_if_exists() }.map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageObjects)),
        )
        .route(
            "/:hash/reissue",
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
            "/:hash/bookmark",
            post(|Path(hash): Path<Hash>| {
                async move { ObjectRef::new(hash).bookmark(BookmarkType::User).mark() }
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
            "/:hash/bookmark",
            get(|Path(hash): Path<Hash>| {
                async move {
                    ObjectRef::new(hash)
                        .bookmark(BookmarkType::User)
                        .is_marked()
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageBookmarks)),
        )
        .route(
            // Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
            "/:hash/bookmark",
            delete(|Path(hash): Path<Hash>| {
                async move { ObjectRef::new(hash).bookmark(BookmarkType::User).unmark() }
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
            "/:hash/reference-count",
            get(|Path(hash): Path<Hash>| {
                async move {
                    ObjectRef::new(hash)
                        .bookmark(BookmarkType::Reference)
                        .get_count()
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::GetObjectStats)),
        )
        .route(
            "/:hash/stats",
            get(|Path(hash): Path<Hash>| {
                async move { ObjectRef::new(hash).statistics() }.map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::GetObjectStats)),
        )
        .route(
            "/:hash/stats/byte-usefulness",
            get(|Path(hash): Path<Hash>| {
                async move {
                    ObjectRef::new(hash).statistics().map(|stats| {
                        stats
                            .map(|stats| stats.byte_usefulness(&crate::models::UsePrior::default()))
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::GetObjectStats)),
        )
}
