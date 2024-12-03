//! Hubs API.

use axum::extract::Path;
use axum::routing::delete;
use axum::routing::get;
use axum::routing::post;
use axum::Json;
use axum::Router;
use futures::FutureExt;
use samizdat_common::address::AddrResolutionMode;
use samizdat_common::address::AddrToResolve;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::Droppable;
use crate::models::Hub;
use crate::security_scope;

/// The entrypoint of the hub API.
pub fn api() -> Router {
    Router::new().merge(hub())
}

fn hub() -> Router {
    #[serde_as]
    #[derive(Deserialize)]
    struct PostHubRequest {
        #[serde_as(as = "DisplayFromStr")]
        address: AddrToResolve,
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

                    hub.insert()?;

                    Ok(PostHubResponse {})
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageHubs)),
        )
        .route(
            // Lists all hubs.
            "/",
            get(|| async move { Hub::get_all() }.map(ApiResponse))
                .layer(security_scope!(AccessRight::ManageHubs)),
        )
        .route(
            // Lists a single hubs.
            "/:hub",
            get(|Path(hub): Path<String>| async move { Hub::get(hub.parse()?) }.map(ApiResponse))
                .layer(security_scope!(AccessRight::ManageHubs)),
        )
        .route(
            "/:hub",
            delete(|Path(hub): Path<String>| {
                async move {
                    let existed = if let Some(hub) = Hub::get(hub.parse()?)? {
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
