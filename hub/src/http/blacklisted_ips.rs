use std::net::IpAddr;

use serde_derive::Deserialize;
use warp::Filter;

use crate::{balanced_or_tree, models::blacklisted_ip::BlacklistedIp};

use super::api_reply;

pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree! { post_blacklisted_ip(), get_blacklisted_ips() }
}

/// Returns all the currently connected IPs to this hub.
fn post_blacklisted_ip(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Debug, Deserialize)]
    struct Request {
        address: IpAddr,
    }

    warp::path("blacklisted-ips")
        .and(warp::post())
        .and(warp::body::json())
        .map(|request: Request| {
            BlacklistedIp::new(request.address).insert()?;
            Ok(())
        })
        .map(api_reply)
}

fn get_blacklisted_ips(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path("blacklisted-ips")
        .and(warp::get())
        .map(|| BlacklistedIp::get_all())
        .map(api_reply)
}
