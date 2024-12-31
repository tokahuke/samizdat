use std::net::IpAddr;

use axum::{
    routing::{get, post},
    Json, Router,
};
use futures::FutureExt;
use samizdat_common::db::{readonly_tx, writable_tx};
use serde_derive::Deserialize;

use crate::{http::ApiResponse, models::blacklisted_ip::BlacklistedIp};

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
                        BlacklistedIp::new(request.address).insert(tx);
                        Ok(())
                    })
                }
                .map(ApiResponse)
            }),
        )
        .route(
            "/",
            get(|| {
                async move { Ok(readonly_tx(|tx| BlacklistedIp::get_all(tx))) }.map(ApiResponse)
            }),
        )
}
