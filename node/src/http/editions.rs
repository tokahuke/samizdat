//! Editions API.

use axum::routing::get;
use axum::Router;
use futures::FutureExt;

use crate::models::Edition;
use crate::{access::AccessRight, security_scope};

use super::ApiResponse;

/// The entrypoint of the series API.
pub fn api() -> Router {
    Router::new().merge(editions())
}

fn editions() -> Router {
    Router::new().route(
        "/",
        get(|| async move { Edition::get_all() }.map(ApiResponse))
            .layer(security_scope!(AccessRight::ManageSeries)),
    )
}
