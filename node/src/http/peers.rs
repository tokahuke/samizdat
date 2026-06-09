//! Connections API.

use std::net::SocketAddr;

use axum::{Router, routing::get};
use futures::FutureExt;
use serde_derive::Serialize;

use crate::{access::AccessRight, security_scope, system::PEER_CONNECTIONS};

use super::ApiResponse;

/// The entrypoint of the hub API.
pub fn api() -> Router {
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

    Router::new().route(
        // Lists all hubs.
        "/",
        get(|| {
            async move {
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
            }
            .map(ApiResponse)
        })
        .layer(security_scope!(read; AccessRight::ManageHubs)),
    )
}
