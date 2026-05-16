use std::net::IpAddr;

use axum::{
    routing::{get, post},
    Json, Router,
};
use futures::FutureExt;
use samizdat_common::db::{readonly_tx, writable_tx};
use serde_derive::Deserialize;

use crate::{http::ApiResponse, models::BlacklistedIp};

// TODO(hub-admin-auth): these endpoints are reachable by anything on the local
// host (loopback bind + `deny_outside_requests`). On a multi-tenant host any local
// process can blacklist arbitrary IPs and there is no DELETE route to undo it.
// Add an admin bearer-token middleware (env var `SAMIZDAT_HUB_ADMIN_TOKEN` or
// similar) and a DELETE route before exposing the hub to shared infrastructure.
pub fn api() -> Router {
    #[derive(Debug, Deserialize)]
    struct PostBlacklistedIPRequest {
        address: IpAddr,
    }
    Router::new()
        .route(
            "/",
            post(|Json(request): Json<PostBlacklistedIPRequest>| {
                async move {
                    writable_tx(|tx| {
                        BlacklistedIp::new(request.address).insert(tx)?;
                        Ok(())
                    })
                }
                .map(ApiResponse)
            }),
        )
        .route(
            "/",
            get(|| {
                async move { readonly_tx(|tx| BlacklistedIp::get_all(tx)) }.map(ApiResponse)
            }),
        )
}
