//! Connections API.

use std::net::SocketAddr;

use serde_derive::Serialize;
use warp::Filter;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::system::PEER_CONNECTIONS;

use super::async_api_reply;
use super::authenticate;

/// The entrypoint of the hub API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // Connection CRUD
        get_peers(),
    )
}

#[derive(Debug, Serialize)]
struct GetPeerResponse {
    addr: SocketAddr,
    is_closed: bool,
}

/// Lists all hubs.
fn get_peers() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_peers")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|| async move {
            let peers = PEER_CONNECTIONS
                .read()
                .await
                .iter()
                .map(|(addr, multiplexed)| GetPeerResponse {
                    addr: *addr,
                    is_closed: multiplexed.is_closed(),
                })
                .collect::<Vec<_>>();

            Ok(peers)
        })
        .and_then(async_api_reply)
}
