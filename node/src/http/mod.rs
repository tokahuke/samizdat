//! HTTP API for the Samizdat Node.

mod auth;
mod collections;
mod connections;
mod editions;
mod ethereum_provider;
mod hubs;
mod identities;
mod kvstore;
mod objects;
mod peers;
mod redirects;
mod resolvers;
mod series;
mod series_owners;
mod subscriptions;

use std::{
    convert::Infallible,
    net::{Ipv6Addr, SocketAddr},
    num::ParseIntError,
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, Request},
    middleware::Next,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures::FutureExt;
use http::request::Parts;
use redirects::redirect_request;

use crate::cli;

/// Gets the corresponding HTTP status code for a Samizdat error.
fn error_status_code(err: &crate::Error) -> http::StatusCode {
    match err {
        crate::Error::Message(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Rpc(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::Base64(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Io(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::BadHashLength(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Bincode(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::QuicConnectionError(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::AllCandidatesFailed => http::StatusCode::BAD_GATEWAY,
        crate::Error::InvalidCollectionItem => http::StatusCode::BAD_REQUEST,
        crate::Error::InvalidEdition => http::StatusCode::BAD_REQUEST,
        crate::Error::DifferentPublicKeys => http::StatusCode::BAD_REQUEST,
        crate::Error::NoHeaderRead => http::StatusCode::INTERNAL_SERVER_ERROR,
        _ => http::StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Represents a timeout value extracted from the X-Samizdat-Timeout header.
struct SamizdatTimeoutRejection(ParseIntError);

impl IntoResponse for SamizdatTimeoutRejection {
    fn into_response(self) -> Response {
        Response::builder()
            .status(400)
            .body(format!("Bad X-Samizdat-Timout header value: {}", self.0).into())
            .expect("can build error response")
    }
}

/// Represents a parsed timeout duration from the X-Samizdat-Timeout header.
struct SamizdatTimeout(Duration);

impl<S: Send + Sync> FromRequestParts<S> for SamizdatTimeout {
    type Rejection = SamizdatTimeoutRejection;
    async fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> Result<SamizdatTimeout, Self::Rejection> {
        parts
            .headers
            .get("X-Samizdat-Timeout")
            .map(|header| {
                String::from_utf8_lossy(header.as_bytes())
                    .parse::<u64>()
                    .map(Duration::from_secs)
            })
            .unwrap_or(Ok(Duration::from_secs(10)))
            .map(SamizdatTimeout)
            .map_err(SamizdatTimeoutRejection)
    }
}

/// Represents the Content-Type header value for requests.
struct ContentType(String);

impl<S: Send + Sync> FromRequestParts<S> for ContentType {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<ContentType, Self::Rejection> {
        Ok(parts
            .headers
            .get("Content-Type")
            .map(|header| String::from_utf8_lossy(header.as_bytes()).into_owned())
            .map(ContentType)
            .unwrap_or_else(|| ContentType("application/octet-stream".to_owned())))
    }
}

/// The standardized JSON reply for the API.
pub struct ApiResponse<T>(Result<T, crate::Error>);

impl<T> IntoResponse for ApiResponse<T>
where
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        let status = self
            .0
            .as_ref()
            .map_err(error_status_code)
            .err()
            .unwrap_or_default();
        let json = self.0.map_err(|err| err.to_string());

        Response::builder()
            .status(status)
            .body(
                serde_json::to_string(&json)
                    .expect("can serialize API response")
                    .into(),
            )
            .expect("can create API response")
    }
}

/// A response that is not a response from the API directly, but "anything else". Used
/// mainly for serving content.
pub struct PageResponse(Result<Response, crate::Error>);

impl IntoResponse for PageResponse {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(response) => response,
            Err(err) => Response::builder()
                .status(error_status_code(&err))
                .body(err.to_string().into())
                .expect("can build error response"),
        }
    }
}

/// The entrypoint of the Samizdat node public HTTP API.
fn api() -> Router {
    Router::new()
        .merge(identities::api())
        .nest("/_kvstore", kvstore::api())
        .nest("/_objects", objects::api())
        .nest("/_collections", collections::api())
        .nest("/_series", series::api())
        .nest("/_series-owners", series_owners::api())
        .nest("/_editions", editions::api())
        .nest("/_subscriptions", subscriptions::api())
        .nest("/_ethereum-provider", ethereum_provider::api())
        .nest("/_auth", auth::api())
        .nest("/_hubs", hubs::api())
        .nest("/_connections", connections::api())
        .nest("/_peers", peers::api())
        .nest("/_vacuum", vacuum())
}

/// Creates a router for vacuum-related endpoints.
///
/// Provides endpoints for triggering manual vacuum operations and
/// flushing all data.
fn vacuum() -> Router {
    Router::new()
        .route(
            "/",
            post(|| async { crate::vacuum::vacuum() }.map(ApiResponse)),
        )
        .route(
            "/flush-all",
            post(|| {
                async {
                    crate::vacuum::flush_all();
                    Ok(())
                }
                .map(ApiResponse)
            }),
        )
}

/// Middleware function to restrict access to only local connections.
///
/// # Arguments
/// * `addr` - Socket address information of the incoming connection
/// * `request` - The incoming HTTP request
/// * `next` - The next middleware in the chain
///
/// # Returns
/// Returns a 403 Forbidden response for non-loopback addresses, otherwise
/// continues the middleware chain.
async fn deny_outside_requests(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if !addr.ip().to_canonical().is_loopback() {
        return Response::builder()
            .status(http::StatusCode::FORBIDDEN)
            .body("403 Forbidden".into())
            .expect("can build stadard error message");
    }

    next.run(request).await
}

/// Runs the HTTP API server.
pub async fn serve() -> Result<(), crate::Error> {
    let server = Router::new()
        .route("/", get(|| async { Html(include_str!("../index.html")) }))
        .merge(api())
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::middleware::from_fn(deny_outside_requests))
                .layer(axum::middleware::from_fn(redirect_request))
                .layer(tower_http::trace::TraceLayer::new_for_http()),
        );

    axum::serve(
        tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, cli().port)).await?,
        server.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
