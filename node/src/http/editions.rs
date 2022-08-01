//! Editions API.

use warp::Filter;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::models::Edition;

use super::{api_reply, authenticate};

/// The entrypoint of the series API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(get_editions(),)
}

/// Lists all series owners.
fn get_editions() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_editions")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageSeries]))
        .map(|| Edition::get_all())
        .map(api_reply)
}
