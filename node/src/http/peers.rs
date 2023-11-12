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
enum PeerStatus {
    Connecting,
    Connected,
    Closed,
    Failed,
}

#[derive(Debug, Serialize)]
struct GetPeerResponse {
    addr: SocketAddr,
    status: PeerStatus,
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
                .map(|(addr, multiplexed)| {
                    multiplexed
                        .try_lock()
                        .ok()
                        .map(|guard| {
                            if let Some(m) = guard.as_ref() {
                                GetPeerResponse {
                                    addr: *addr,
                                    status: if m.is_closed() {
                                        PeerStatus::Closed
                                    } else {
                                        PeerStatus::Connected
                                    },
                                }
                            } else {
                                GetPeerResponse {
                                    addr: *addr,
                                    status: PeerStatus::Failed,
                                }
                            }
                        })
                        .unwrap_or_else(|| GetPeerResponse {
                            addr: *addr,
                            status: PeerStatus::Connecting,
                        })
                })
                .collect::<Vec<_>>();

            Ok(peers)
        })
        .and_then(async_api_reply)
}
