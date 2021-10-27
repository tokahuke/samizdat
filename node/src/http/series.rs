use serde_derive::Deserialize;
use std::time::Duration;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::Key;

use crate::balanced_or_tree;
use crate::models::{CollectionRef, Dropable, SeriesOwner, SeriesRef};

use super::resolvers::resolve_series;
use super::{reply, returnable, tuple};

/// The entrypoint of the Samizdat node HTTP API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        get_series_owner(),
        get_series_owners(),
        post_series_owner(),
        delete_series_owner(),
        post_series(),
        get_item_by_series()
    )
}

/// Creates a new series owner, i.e., a public-private keypair that allows one to push new
/// colletions to a series.
fn post_series_owner() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    #[derive(Deserialize)]
    struct Keypair {
        public_key: String,
        private_key: String,
    }

    #[derive(Deserialize)]
    struct Request {
        series_owner_name: String,
        keypair: Option<Keypair>,
        #[serde(default)]
        is_draft: bool,
    }

    warp::path!("_seriesowners")
        .and(warp::post())
        .and(warp::body::json())
        .map(|request: Request| {
            let series_owner = if let Some(Keypair {
                public_key,
                private_key,
            }) = request.keypair
            {
                SeriesOwner::import(
                    &request.series_owner_name,
                    public_key.parse()?,
                    private_key.parse()?,
                    Duration::from_secs(3_600),
                    request.is_draft,
                )
            } else {
                SeriesOwner::create(
                    &request.series_owner_name,
                    Duration::from_secs(3_600),
                    request.is_draft,
                )
            };

            Ok(returnable::Json(series_owner?))
        })
        .map(reply)
}

/// Removes a series owner
fn delete_series_owner(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::delete())
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            Ok(maybe_owner.map(|owner| owner.drop_if_exists()))
        })
        .map(reply)
}

/// Gets information associates with a series owner
fn get_series_owner() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_seriesowners" / String)
        .and(warp::get())
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            Ok(maybe_owner.map(|owner| owner.series().to_string()))
        })
        .map(reply)
}

/// Lists all series owners.
fn get_series_owners() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_seriesowners")
        .and(warp::get())
        .map(|| {
            let series = SeriesOwner::get_all()?;
            Ok(returnable::Json(series))
        })
        .map(reply)
}

/// Pushes a new colletion to the series owner, creating a new series item.
fn post_series() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        collection: String,
        #[serde(default)]
        #[serde(with = "humantime_serde")]
        ttl: Option<std::time::Duration>,
    }

    warp::path!("_seriesowners" / String / "collections")
        .and(warp::post())
        .and(warp::body::json())
        .map(|series_owner_name: String, request: Request| {
            if let Some(series_owner) = SeriesOwner::get(&series_owner_name)? {
                let series = series_owner
                    .advance(CollectionRef::new(request.collection.parse()?), request.ttl)?;
                Ok(Some(returnable::Json(series)))
            } else {
                Ok(None)
            }
        })
        .map(reply)
}

/// Gets the content of a collection item using the series public key. This will give the
/// best-effort latest version for this item.
fn get_item_by_series(
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
