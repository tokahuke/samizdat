use serde_derive::Deserialize;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::Hash;

use crate::access_token::AccessRight;
use crate::balanced_or_tree;
use crate::models::{CollectionRef, ItemPathBuf, ObjectRef};

use super::resolvers::resolve_item;
use super::{authenticate, reply, returnable, tuple};

/// The entrypoint of the collection public API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(get_item(), post_collection())
}

/// Uploads a new collection.
pub fn post_collection(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        #[serde(default)]
        is_draft: bool,
        hashes: Vec<(String, String)>,
    }

    warp::path!("_collections")
        .and(warp::post())
        .and(authenticate([AccessRight::ManageCollections]))
        .and(warp::body::json())
        .map(|request: Request| {
            let collection = CollectionRef::build(
                request.is_draft,
                request
                    .hashes
                    .into_iter()
                    .map(|(name, hash)| {
                        Ok((ItemPathBuf::from(name), ObjectRef::new(hash.parse()?)))
                    })
                    .collect::<Result<Vec<_>, crate::Error>>()?,
            )?;
            Ok(returnable::Return {
                content_type: "text/plain".to_owned(),
                status_code: http::StatusCode::OK,
                content: collection.hash().to_string().as_bytes().to_vec(),
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
            Ok(resolve_item(locator, []).await?) as Result<_, warp::Rejection>
        })
        .map(tuple)
}
