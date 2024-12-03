//! HTTP API for the Samizdat Hub.

mod blacklisted_ips;

use axum::extract::{ConnectInfo, Query, Request};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::{FutureExt, StreamExt};
use serde_derive::Deserialize;
use std::net::{Ipv6Addr, SocketAddr};

use crate::rpc::node_sampler::QuerySampler;
use crate::rpc::ROOM;
use crate::CLI;

/// Mapping of Samizdat errors into HTTP status codes.
fn error_status_code(err: &crate::Error) -> http::StatusCode {
    match err {
        crate::Error::Message(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Rpc(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::Base64(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Db(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
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

/// Serves the Samizdat hub HTTP API.
pub async fn serve() -> Result<(), crate::Error> {
    let server = Router::new()
        .route("/", get(|| async { Html(include_str!("../index.html")) }))
        .merge(api())
        .layer(axum::middleware::from_fn(deny_outside_requests));

    axum::serve(
        tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, CLI.http_port)).await?,
        server.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// All the endpoints for the Samizdat HTTP API.
fn api() -> Router {
    Router::new()
        .nest("connected-ips", connected_ips())
        .nest("resolution-order", resolution_order())
        .nest("blacklisted-ips", blacklisted_ips::api())
}

/// Returns all the currently connected IPs to this hub.
fn connected_ips() -> Router {
    Router::new().route(
        "/",
        get(|| {
            async move {
                let ips = ROOM
                    .raw_participants()
                    .await
                    .iter()
                    .map(|(addr, _)| *addr)
                    .collect::<Vec<_>>();
                Ok(ips)
            }
            .map(ApiResponse)
        }),
    )
}

/// Gets the current resolution order of the peers, that is, the order in which peers
/// will be queried.
fn resolution_order() -> Router {
    #[derive(Deserialize)]
    struct QueryParameters {
        addr: SocketAddr,
    }

    Router::new().route(
        "/",
        get(|Query(QueryParameters { addr }): Query<QueryParameters>| {
            async move {
                let resolution_order = ROOM
                    .stream_peers(QuerySampler, addr)
                    .await
                    .map(|(peer_ip, _)| peer_ip)
                    .collect::<Vec<_>>()
                    .await;
                Ok(resolution_order)
            }
            .map(ApiResponse)
        }),
    )
}
