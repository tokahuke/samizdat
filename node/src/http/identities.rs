use std::convert::identity;

use serde_derive::Deserialize;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::Hash;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::models::IdentityRef;

use super::resolvers::resolve_identity;
use super::{api_reply, authenticate, tuple};

/// The entrypoint of the object API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // // Identity CRUD
        // get_identity(),
        // post_identity(),
        // delete_identity(),
        // Query item using identity
        get_item(),
    )
}

/// Gets the contents of an item using identity.
fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!(IdentityRef / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|identity, name: Tail| async move {
            Ok(resolve_identity(identity, name.as_str().into(), []).await?)
                as Result<_, warp::Rejection>
        })
        .map(tuple)
}
