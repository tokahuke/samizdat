//! Editions API.

use axum::{Router, routing::get};
use futures::FutureExt;
use samizdat_common::db::readonly_tx;

use crate::{access::AccessRight, models::Edition, security_scope};

use super::ApiResponse;

/// The entrypoint of the series API.
pub fn api() -> Router {
    Router::new().merge(editions())
}

fn editions() -> Router {
    Router::new().route(
        "/",
        get(|| async move { readonly_tx(|tx| Edition::get_all(tx)) }.map(ApiResponse))
            .layer(security_scope!(read; AccessRight::ManageSeries)),
    )
}
