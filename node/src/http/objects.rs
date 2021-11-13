use serde_derive::Deserialize;
use warp::Filter;

use samizdat_common::Hash;

use crate::balanced_or_tree;
use crate::models::{BookmarkType, Dropable, ObjectHeader, ObjectRef};

use super::resolvers::resolve_object;
use super::{async_reply, reply, returnable, tuple};

/// The entrypoint of the Samizdat node HTTP API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // Object CRUD
        get_object(),
        post_object(),
        delete_object(),
        // Bookmark CRUD:
        get_bookmark(),
        post_bookmark(),
        delete_bookmark(),
        // Statistics:
        get_stats(),
        get_byte_usefulness(),
        // Utils:
        post_reissue(),
        get_reference_count(),
    )
}

/// Gets the contents of an object.
fn get_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::get())
        .and_then(|hash: Hash| async move {
            Ok(resolve_object(ObjectRef::new(hash), vec![]).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}

/// Uploads a new object to the database.
fn post_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    #[serde(rename = "kebab-case")]
    struct Query {
        #[serde(default)]
        bookmark: bool,
        #[serde(default)]
        is_draft: bool,
    }

    warp::path!("_objects")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::query())
        .and(warp::body::bytes())
        .map(
            |content_type: String, query: Query, bytes: bytes::Bytes| async move {
                let header = ObjectHeader::new(content_type, query.is_draft)?;
                let object =
                    ObjectRef::build(header, query.bookmark, bytes.into_iter().map(Result::Ok))?;
                Ok(object.hash().to_string())
            },
        )
        .and_then(async_reply)
}

/// Explicitly deletes an object from the local database. This does not have the
/// effect of deleting it from the whole network. It only clears a local buffer.
fn delete_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).drop_if_exists())
        .map(reply)
}

/// Bookmarks an object. This will prevent the object from being automatically removed
/// by the vacuum daemon.
fn post_bookmark() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "bookmark")
        .and(warp::post())
        .map(|hash| ObjectRef::new(hash).bookmark(BookmarkType::User).mark())
        .map(reply)
}

/// Returns whether an object is bookmarked or not.
///
/// # Warning
///
/// By now, this returns `200 OK` even if the object does not exist.
fn get_bookmark() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "bookmark")
        .and(warp::get())
        .map(|hash| {
            ObjectRef::new(hash)
                .bookmark(BookmarkType::User)
                .is_marked()
                .map(returnable::Json)
        })
        .map(reply)
}

/// Returns the internal reference count on the object.
///
/// # Warning
///
/// By now, this returns `200 OK` even if the object does not exist.
fn get_reference_count() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "reference-count")
        .and(warp::get())
        .map(|hash| {
            ObjectRef::new(hash)
                .bookmark(BookmarkType::Reference)
                .get_count()
                .map(returnable::Json)
        })
        .map(reply)
}

/// Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
fn delete_bookmark() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_objects" / Hash / "bookmark")
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).bookmark(BookmarkType::User).unmark())
        .map(reply)
}

/// Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
fn get_stats() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "stats")
        .and(warp::get())
        .map(|hash| ObjectRef::new(hash).statistics().map(returnable::Json))
        .map(reply)
}

/// Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
fn get_byte_usefulness(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "stats" / "byte-usefulness")
        .and(warp::get())
        .map(|hash| {
            ObjectRef::new(hash)
                .statistics()
                .map(|stats| {
                    stats.map(|stats| stats.byte_usefulness(&crate::models::UsePrior::default()))
                })
                .map(returnable::Json)
        })
        .map(reply)
}

/// Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
fn post_reissue() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    #[serde(rename = "kebab-case")]
    struct Query {
        #[serde(default)]
        bookmark: bool,
    }

    warp::path!("_objects" / Hash / "reissue")
        .and(warp::post())
        .and(warp::query())
        .map(|hash, query: Query| {
            ObjectRef::new(hash)
                .reissue(query.bookmark)
                .map(|reissued| reissued.map(|reissued| reissued.hash().to_string()))
        })
        .map(reply)
}
