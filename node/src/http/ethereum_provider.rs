//! Identities API.

use axum::{
    Json, Router,
    routing::{get, put},
};
use futures::FutureExt;
use samizdat_common::db::{Table as _, readonly_tx};
use serde_derive::{Deserialize, Serialize};

use crate::{db::Table, http::ApiResponse, identity_dapp::identity_provider, security_scope};

/// The entrypoint of the object API.
pub fn api() -> Router {
    #[derive(Deserialize)]
    struct PutEthereumProviderRequest {
        endpoint: String,
    }

    #[derive(Serialize)]
    struct PutEthereumProviderResponse {}

    #[derive(Serialize)]
    struct GetEthereumProviderResponse {
        endpoint: String,
    }

    Router::new()
        .route(
            // Sets the Ethereum Network Provider to be used.
            "/",
            put(|Json(request): Json<PutEthereumProviderRequest>| {
                async move {
                    tokio::spawn(async move {
                        identity_provider().set_endpoint(&request.endpoint).await
                    });
                    Ok(PutEthereumProviderResponse {})
                }
                .map(ApiResponse)
            })
            .layer(security_scope!()),
        )
        .route(
            // Gets the Ethereum Network Provider to be used.
            "/",
            get(|| {
                async move {
                    Ok(GetEthereumProviderResponse {
                        endpoint: readonly_tx(|tx| {
                            Table::Global.get(tx, "ethereum_provider_endpoint", |e| {
                                Ok(String::from_utf8_lossy(e).into_owned())
                            })
                        })?
                        .unwrap_or_else(|| {
                            samizdat_common::blockchain::DEFAULT_PROVIDER_ENDPOINT.to_owned()
                        }),
                    })
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(read;)),
        )
}
