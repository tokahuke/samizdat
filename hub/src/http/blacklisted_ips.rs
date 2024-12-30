use std::net::IpAddr;

use axum::{
    routing::{get, post},
    Json, Router,
};
use futures::FutureExt;
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
                async move { BlacklistedIp::new(request.address).insert() }.map(ApiResponse)
            }),
        )
        .route(
            "/",
            get(|| async move { Ok(BlacklistedIp::get_all()) }.map(ApiResponse)),
        )
}
