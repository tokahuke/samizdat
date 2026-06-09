//! Hubs API.

use axum::{
    Json, Router,
    extract::Path,
    routing::{delete, get, post},
};
use futures::FutureExt;
use samizdat_common::{
    address::AddrResolutionMode,
    db::{Droppable, readonly_tx, writable_tx},
};
use serde_derive::{Deserialize, Serialize};

use crate::{access::AccessRight, http::ApiResponse, models::Hub, security_scope};

/// The entrypoint of the hub API.
pub fn api() -> Router {
    Router::new().merge(hub())
}

fn hub() -> Router {
    #[derive(Deserialize)]
    struct PostHubRequest {
        address: String,
        resolution_mode: AddrResolutionMode,
    }

    #[derive(Serialize)]
    struct PostHubResponse {}

    Router::new()
        .route(
            "/",
            post(|Json(request): Json<PostHubRequest>| {
                async move {
                    let hub = Hub {
                        address: request.address,
                        resolution_mode: request.resolution_mode,
                    };

                    writable_tx(|tx| hub.insert(tx))?;

                    Ok(PostHubResponse {})
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageHubs)),
        )
        .route(
            // Lists all hubs.
            "/",
            get(|| async move { readonly_tx(|tx| Hub::get_all(tx)) }.map(ApiResponse))
                .layer(security_scope!(read; AccessRight::ManageHubs)),
        )
        .route(
            // Lists a single hubs.
            "/{hub}",
            get(|Path(hub): Path<String>| {
                async move { readonly_tx(|tx| Hub::get(tx, &hub)) }.map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::ManageHubs)),
        )
        .route(
            "/{hub}",
            delete(|Path(hub): Path<String>| {
                async move {
                    let existed = if let Some(hub) = readonly_tx(|tx| Hub::get(tx, &hub))? {
                        hub.drop_if_exists()?;
                        true
                    } else {
                        false
                    };

                    Ok(existed)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageHubs)),
        )
}
