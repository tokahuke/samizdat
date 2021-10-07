//! HTTP API for the Samizdat Node.

mod resolvers;
mod returnable;

pub use returnable::{Json, Return, Returnable};

use futures::stream;
use serde_derive::Deserialize;
use std::time::Duration;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::{Hash, Key};

use crate::balanced_or_tree;
use crate::models::{BookmarkType, CollectionRef, Dropable, ObjectRef, SeriesOwner, SeriesRef};

use resolvers::{resolve_item, resolve_object, resolve_series};

/// Transforms a `Result<T, crate::Error>` into a Warp reply.
fn reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: Returnable,
{
    warp::reply::with_header(
        warp::reply::with_status(t.render().into_owned(), t.status_code()),
        http::header::CONTENT_TYPE,
        &*t.content_type(),
    )
}

/// Transforms a `Result<T, crate::Error>` future into a Warp reply.
async fn async_reply<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

/// Utility to create a tuple of one value _very explicitely_.
fn tuple<T>(t: T) -> (T,) {
    (t,)
}

/// Optionaly implements the "tilde redirect". Similarly to Unix platforms, the `~`
/// represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let entity_type = split.next()?;
    let entity_identifier = split.next()?;

    let mut found_tilde = false;
    for item in &mut split {
        if item == "~" {
            found_tilde = true;
            break;
        }
    }

    if found_tilde {
        let tail = split.collect::<Vec<_>>().join("/");
        Some(format!("/{}/{}/{}", entity_type, entity_identifier, tail))
    } else {
        None
    }
}

/// Optionally redirects a "home path" without trailing slash to the same path with
/// trailing slash.
fn maybe_redirect_base(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let entity_type = split.next()?;
    let entity_identifier = split.next()?;
    let is_redirectable_entity = entity_type == "_collections" || entity_type == "_series";

    if split.next().is_none() && is_redirectable_entity {
        Some(format!("/{}/{}/", entity_type, entity_identifier))
    } else {
        None
    }
}

/// Removes empty path segments from the URL.
fn maybe_redirect_empty(path: &str) -> Option<String> {
    if path.contains("//") {
        let split = path.split('/');
        let without_initial_slash = split
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        Some(format!("/{}", without_initial_slash))
    } else {
        None
    }
}

/// The entrypoint of the Samizdat node HTTP API.
pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        general_redirect(),
        get_object(),
        post_object(),
        delete_object(),
        post_bookmark(),
        get_bookmark(),
        delete_bookmark(),
        post_collection(),
        get_collection_list(),
        get_item(),
        get_series_owner(),
        get_series_owners(),
        post_series_owner(),
        post_series(),
        get_item_by_series()
    )
}

/// Does all the redirection dances and shenenigans.
pub fn general_redirect(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::path::tail())
        .and_then(|path: warp::path::Tail| async move {
            let maybe_redirect = maybe_redirect_tilde(path.as_str())
                .or_else(|| maybe_redirect_base(path.as_str()))
                .or_else(|| maybe_redirect_empty(path.as_str()));

            if let Some(location) = maybe_redirect {
                log::info!("location {}", location);
                let uri = location
                    .parse::<http::uri::Uri>()
                    .expect("bad route on tilde redirect");
                Ok(warp::redirect(uri))
            } else {
                Err(warp::reject::reject())
            }
        })
}

/// Gets the contents of an object.
pub fn get_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::get())
        .and_then(|hash: Hash| async move {
            Ok(resolve_object(ObjectRef::new(hash), vec![]).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}

/// Uploads a new object to the database.
pub fn post_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    #[derive(Deserialize)]
    struct Query {
        #[serde(default)]
        bookmark: bool,
    }

    warp::path!("_objects")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::query())
        .and(warp::body::bytes())
        .map(
            |content_type: String, query: Query, bytes: bytes::Bytes| async move {
                let (_, object) = ObjectRef::build(
                    content_type,
                    bytes.len(),
                    query.bookmark,
                    stream::iter(bytes.into_iter().map(|byte| Ok(byte))),
                )
                .await?;
                Ok(object.hash().to_string())
            },
        )
        .and_then(async_reply)
}

/// Explicitely deletes an object from the database.
pub fn delete_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_objects" / Hash)
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).drop_if_exists())
        .map(reply)
}

/// Bookmarks an object. This will prevent the object from being automatically removed
/// by the vacuum daemon.
pub fn post_bookmark() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
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
pub fn get_bookmark() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_objects" / Hash / "bookmark")
        .and(warp::get())
        .map(|hash| {
            ObjectRef::new(hash)
                .bookmark(BookmarkType::User)
                .is_marked()
                .map(Json)
        })
        .map(reply)
}

/// Removes the bookmark from an object, allowing the vacuum daemon to gobble it up.
pub fn delete_bookmark(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash / "bookmark")
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).bookmark(BookmarkType::User).unmark())
        .map(reply)
}

/// Uploads a new collection.
pub fn post_collection(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_collections")
        .and(warp::post())
        .and(warp::body::json())
        .map(|hashes: Vec<(String, String)>| {
            let collection = CollectionRef::build(
                hashes
                    .into_iter()
                    .map(|(name, hash)| Ok((name, ObjectRef::new(hash.parse()?))))
                    .collect::<Result<Vec<_>, crate::Error>>()?,
            )?;
            Ok(Return {
                content_type: "text/plain".to_owned(),
                status_code: http::StatusCode::OK,
                content: collection.hash.to_string().as_bytes().to_vec(),
            })
        })
        .map(reply)
}

/// Gets the contents of a collection item.
pub fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_collections" / Hash / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|hash: Hash, name: Tail| async move {
            let collection = CollectionRef::new(hash);
            let path = name.as_str().into();
            let locator = collection.locator_for(path);
            Ok(resolve_item(locator).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}

/// Lists all the items currently in the database. This is akin to a sitemap.
pub fn get_collection_list(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_collections" / Hash / "_list")
        .and(warp::get())
        .map(|hash: Hash| {
            let collection = CollectionRef::new(hash);
            Ok(Json(collection.list().collect::<Vec<_>>())) as Result<_, crate::Error>
        })
        .map(reply)
}

/// Creates a new series owner, i.e., a public-private keypair that allows one to push new
/// colletions to a series.
pub fn post_series_owner(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::post())
        .map(|series_owner_name: String| {
            let series_owner = SeriesOwner::create(&series_owner_name, Duration::from_secs(3_600))?;
            Ok(Json(series_owner))
        })
        .map(reply)
}

/// Gets information associates with a series owner
pub fn get_series_owner(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::get())
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            Ok(maybe_owner.map(|owner| owner.series().to_string()))
        })
        .map(reply)
}

/// Lists all series owners.
pub fn get_series_owners(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners")
        .and(warp::get())
        .map(|| {
            let series = SeriesOwner::get_all()?;
            Ok(returnable::Json(series))
        })
        .map(reply)
}

/// Pushes a new colletion to the series owner, creating a new series item.
pub fn post_series() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    #[derive(Deserialize)]
    struct Request {
        #[serde(default)]
        #[serde(with = "humantime_serde")]
        ttl: Option<std::time::Duration>,
    }

    warp::path!("_seriesowners" / String / "collections" / Hash)
        .and(warp::post())
        .and(warp::query())
        .map(|series_owner_name: String, collection, request: Request| {
            if let Some(series_owner) = SeriesOwner::get(&series_owner_name)? {
                let series = series_owner.advance(CollectionRef::new(collection), request.ttl)?;
                Ok(Some(returnable::Json(series)))
            } else {
                Ok(None)
            }
        })
        .map(reply)
}

/// Gets the content of a collection item using the series public key. This will give the
/// best-effort latest version for this item.
pub fn get_item_by_series(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_series" / Key / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|series_key: Key, name: Tail| async move {
            let series = SeriesRef::new(series_key);
            Ok(resolve_series(series, name.as_str().into()).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}
