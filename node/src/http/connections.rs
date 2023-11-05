//! Connections API.

use std::net::SocketAddr;

use serde_derive::Serialize;
use warp::Filter;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::system::ConnectionStatus;

use super::async_api_reply;
use super::authenticate;

/// The entrypoint of the hub API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // Connection CRUD
        get_connections(),
    )
}

#[derive(Debug, Serialize)]
struct GetConnectionResponse {
    name: String,
    status: ConnectionStatus,
    direct_addr: SocketAddr,
    reverse_addr: SocketAddr,
}

/// Lists all hubs.
fn get_connections() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_connections")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|| async move {
            let connections = crate::hubs()
                .snapshot()
                .await
                .into_iter()
                .map(|connection| GetConnectionResponse {
                    name: connection.name().to_string(),
                    status: connection.status(),
                    direct_addr: connection.address().direct_addr(),
                    reverse_addr: connection.address().reverse_addr(),
                })
                .collect::<Vec<_>>();

            Ok(connections)
        })
        .and_then(async_api_reply)
}
