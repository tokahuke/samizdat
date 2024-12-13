//! Connections API.

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;
use futures::FutureExt;
use serde_derive::Serialize;

use crate::access::AccessRight;
use crate::security_scope;
use crate::system::ConnectionStatus;

use super::ApiResponse;

/// The entrypoint of the hub API.
pub fn api() -> Router {
    Router::new().merge(connections())
}

#[derive(Debug, Serialize)]
struct GetConnectionResponse {
    name: String,
    status: ConnectionStatus,
    addr: SocketAddr,
}

fn connections() -> Router {
    Router::new().route(
        // Lists all hubs.
        "/",
        get(|| {
            async move {
                let connections = crate::hubs()
                    .snapshot()
                    .await
                    .into_iter()
                    .map(|connection| GetConnectionResponse {
                        name: connection.name().to_string(),
                        status: connection.status(),
                        addr: connection.address(),
                    })
                    .collect::<Vec<_>>();

                Ok(connections)
            }
            .map(ApiResponse)
        })
        .layer(security_scope!(AccessRight::ManageHubs)),
    )
}
