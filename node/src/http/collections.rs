use warp::path::Tail;
use warp::Filter;

use samizdat_common::Hash;

use crate::balanced_or_tree;
use crate::models::{CollectionRef, ItemPathBuf, ObjectRef};

use super::resolvers::resolve_item;
use super::{reply, returnable, tuple};

/// The entrypoint of the Samizdat node HTTP API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(post_collection(), get_collection_list(), get_item(),)
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
                    .map(|(name, hash)| {
                        Ok((ItemPathBuf::from(name), ObjectRef::new(hash.parse()?)))
                    })
                    .collect::<Result<Vec<_>, crate::Error>>()?,
            )?;
            Ok(returnable::Return {
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
            Ok(returnable::Json(collection.list().collect::<Vec<_>>())) as Result<_, crate::Error>
        })
        .map(reply)
}
