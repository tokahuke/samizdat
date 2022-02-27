use serde_derive::Deserialize;
use std::time::Duration;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::Key;

use crate::access::AccessRight;
use crate::models::{CollectionRef, Dropable, SeriesOwner, SeriesRef};
use crate::{balanced_or_tree, hubs};

use super::resolvers::resolve_series;
use super::{api_reply, authenticate, tuple};

/// The entrypoint of the series API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        get_edition(),
        get_series_owner(),
        get_series_owners(),
        post_series_owner(),
        delete_series_owner(),
        post_edition(),
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
        #[serde(default)]
        keypair: Option<Keypair>,
        #[serde(default)]
        is_draft: bool,
    }

    warp::path!("_seriesowners")
        .and(warp::post())
        .and(authenticate([AccessRight::ManageSeries]))
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

            Ok(series_owner?)
        })
        .map(api_reply)
}

/// Removes a series owner
fn delete_series_owner(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::delete())
        .and(authenticate([AccessRight::ManageSeries]))
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            let existed = maybe_owner
                .map(|owner| owner.drop_if_exists())
                .transpose()?
                .is_some();
            Ok(existed)
        })
        .map(api_reply)
}

/// Gets information associates with a series owner
fn get_series_owner() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_seriesowners" / String)
        .and(warp::get())
        .and(authenticate([AccessRight::ManageSeries]))
        .map(|series_owner_name: String| {
            let maybe_owner = SeriesOwner::get(&series_owner_name)?;
            Ok(maybe_owner.map(|owner| owner))
        })
        .map(api_reply)
}

/// Lists all series owners.
fn get_series_owners() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_seriesowners")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageSeries]))
        .map(|| SeriesOwner::get_all())
        .map(api_reply)
}

/// Pushes a new collection to the series owner, creating a new edition.
fn post_edition() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        collection: String,
        #[serde(default)]
        #[serde(with = "humantime_serde")]
        ttl: Option<std::time::Duration>,
        #[serde(default)]
        no_annouce: bool,
    }

    warp::path!("_seriesowners" / String / "editions")
        .and(warp::post())
        .and(authenticate([AccessRight::ManageSeries]))
        .and(warp::body::json())
        .map(|series_owner_name: String, request: Request| {
            if let Some(series_owner) = SeriesOwner::get(&series_owner_name)? {
                let edition = series_owner
                    .advance(CollectionRef::new(request.collection.parse()?), request.ttl)?;

                if !request.no_annouce {
                    let announcement = edition.announcement();
                    tokio::spawn({
                        let edition = edition.clone();
                        async move {
                            log::info!("Announcing edition {:?}", edition);
                            hubs().announce_edition(&announcement).await
                        }
                    });
                }

                Ok(edition)
            } else {
                Err(crate::Error::Message(format!(
                    "Series owner {} not found",
                    series_owner_name
                )))
            }
        })
        .map(api_reply)
}

/// Gets the content of a collection item using the series public key. This will give the
/// best-effort latest version for this item.
fn get_edition() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_series" / Key / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|series_key: Key, name: Tail| async move {
            let series = SeriesRef::new(series_key);
            Ok(resolve_series(series, name.as_str().into(), []).await?)
                as Result<_, warp::Rejection>
        })
        .map(tuple)
}
