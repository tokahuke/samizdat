//! Series API.

use axum::extract::Path;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use futures::FutureExt;
use serde_derive::Deserialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use tokio::time::Instant;

use samizdat_common::Key;

use crate::access::AccessRight;
use crate::http::{ApiResponse, PageResponse, SamizdatTimeout};
use crate::models::SeriesRef;
use crate::security_scope;

use super::resolvers::resolve_series;

/// The entrypoint of the series API.
pub fn api() -> Router {
    #[serde_as]
    #[derive(Deserialize)]
    struct SeriesPath {
        #[serde_as(as = "DisplayFromStr")]
        series_key: Key,
        #[serde(default)]
        name: String,
    }

    Router::new()
        .route(
            // Gets the content of a collection item using the series public key. This will give the
            // best-effort latest version for this item.
            "/:series_key/*name",
            get(
                |Path(SeriesPath { series_key, name }): Path<SeriesPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        let series = SeriesRef::new(series_key);
                        resolve_series(series, name.as_str().into(), [], Instant::now() + timeout)
                            .await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            // Gets the content of a collection item using the series public key. This will give the
            // best-effort latest version for this item.
            "/:series_key/",
            get(
                |Path(SeriesPath { series_key, .. }): Path<SeriesPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        let series = SeriesRef::new(series_key);
                        resolve_series(series, "".into(), [], Instant::now() + timeout).await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            // Gets the content of a collection item using the series public key. This will give the
            // best-effort latest version for this item.
            "/:series_key",
            get(
                |Path(SeriesPath { series_key, .. }): Path<SeriesPath>| async move {
                    Redirect::permanent(&format!("{series_key}/"))
                },
            ),
        )
        .route(
            // Lists all known public keys the node has seen, be they locally owned or not.
            "/",
            get(|| async move { SeriesRef::get_all() }.map(ApiResponse))
                .layer(security_scope!(AccessRight::ManageSeries)),
        )
}
