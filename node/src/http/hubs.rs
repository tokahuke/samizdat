//! Hubs API.

use axum::extract::Path;
use axum::routing::delete;
use axum::routing::get;
use axum::routing::post;
use axum::Json;
use axum::Router;
use futures::FutureExt;
use samizdat_common::address::AddrResolutionMode;
use samizdat_common::db::readonly_tx;
use samizdat_common::db::writable_tx;
use samizdat_common::db::Droppable;
use serde_derive::Deserialize;
use serde_derive::Serialize;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::Hub;
use crate::security_scope;

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
                .layer(security_scope!(AccessRight::ManageHubs)),
        )
        .route(
            // Lists a single hubs.
            "/{hub}",
            get(|Path(hub): Path<String>| {
                async move { readonly_tx(|tx| Hub::get(tx, &hub)) }.map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageHubs)),
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
