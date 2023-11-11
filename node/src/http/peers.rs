//! Connections API.

use std::net::SocketAddr;

use serde_derive::Serialize;
use warp::Filter;

use crate::access::AccessRight;
use crate::balanced_or_tree;

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
    hub_name: String,
    addr: SocketAddr,
    is_closed: bool,
}

/// Lists all hubs.
fn get_peers() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_peers")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|| async move {
            let connections = crate::hubs()
                .snapshot()
                .await;
            let mut peers = vec![];

            for connection in connections {
                for peer in connection.peers().await {
                    peers.push(GetPeerResponse {
                        hub_name: connection.name().to_string(),
                        addr: peer.0,
                        is_closed: peer.1,
                    })
                }
            }

            Ok(peers)
        })
        .and_then(async_api_reply)
}
