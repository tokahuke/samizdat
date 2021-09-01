mod resolvers;
mod returnable;

pub use returnable::{Return, Returnable};

use futures::stream;
use serde_derive::Deserialize;
use std::time::Duration;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::{Hash, Key};

use crate::balanced_or_tree;
use crate::models::{CollectionRef, ObjectRef, SeriesOwner, SeriesRef};

use resolvers::{resolve_item, resolve_object, resolve_series};

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

async fn async_reply<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

fn tuple<T>(t: T) -> (T,) {
    (t,)
}

fn maybe_redirect(path: &str) -> Option<String> {
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

pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        tilde_redirect(),
        get_object(),
        post_object(),
        delete_object(),
        post_collection(),
        get_item(),
        get_series_owner(),
        get_series_owners(),
        post_series_owner(),
        post_series(),
        get_item_by_series()
    )
}

pub fn tilde_redirect() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path::tail()
        .and_then(|path: warp::path::Tail| async move {
            if let Some(location) = maybe_redirect(path.as_str()) {
                let uri = location.parse::<http::uri::Uri>().expect("bad route on tilde redirect");
                Ok(warp::redirect(uri))
            } else {
                Err(warp::reject::reject())
            }
        })
}

pub fn get_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_objects" / Hash)
        .and(warp::get())
        .and_then(|hash: Hash| async move {
            Ok(resolve_object(ObjectRef::new(hash)).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}

pub fn post_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_objects")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::body::bytes())
        .map(|content_type: String, bytes: bytes::Bytes| async move {
            let (_, object) = ObjectRef::build(
                content_type,
                bytes.len(),
                stream::iter(bytes.into_iter().map(|byte| Ok(byte))),
            )
            .await?;
            Ok(object.hash.to_string())
        })
        .and_then(async_reply)
}

pub fn delete_object() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_objects" / Hash)
        .and(warp::delete())
        .map(|hash| ObjectRef::new(hash).drop_if_exists())
        .map(reply)
}

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

pub fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_collections" / Hash / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|hash: Hash, name: Tail| async move {
            let collection = CollectionRef::new(hash);
            let locator = collection.locator_for(name.as_str());
            Ok(resolve_item(locator).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}

pub fn post_series_owner(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_seriesowners" / String)
        .and(warp::post())
        .map(|series_owner_name: String| {
            let series_owner = SeriesOwner::create(&series_owner_name, Duration::from_secs(3_600))?;
            Ok(series_owner.series().to_string())
        })
        .map(reply)
}

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

pub fn get_item_by_series(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_series" / Key / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|series_key: Key, name: Tail| async move {
            let series = SeriesRef::new(series_key);
            Ok(resolve_series(series, name.as_str()).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}
