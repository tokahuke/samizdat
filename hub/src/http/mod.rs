//! HTTP API for the Samizdat Hub.

mod blacklisted_ips;

use axum::extract::{ConnectInfo, Query, Request};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures::{FutureExt, StreamExt};
use serde_derive::{Deserialize, Serialize};
use serde_inline_default::serde_inline_default;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use samizdat_common::db::readonly_tx;

use crate::cli::cli;
use crate::models::CandidateLog;
use crate::models::ConnectionLog;
use crate::models::QueryLog;
use crate::models::StatisticsLog;
use crate::models::{Id, Indexable};
use crate::rpc::node_sampler::{QuerySampler, StatisticsType};
use crate::rpc::ROOM;

/// Mapping of Samizdat errors into HTTP status codes.
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
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::middleware::from_fn(deny_outside_requests))
                .layer(tower_http::trace::TraceLayer::new_for_http()),
        );

    // Bind to loopback only. The admin plane is operator-facing; remote operators
    // should tunnel in (ssh -L, wireguard) rather than expose this port. Combined
    // with `deny_outside_requests`, this is defense in depth: a future middleware
    // reorder, an added route mounted before `.layer`, or a reverse proxy in front
    // can't accidentally publish the admin API.
    axum::serve(
        tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, cli().http_port)).await?,
        server.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// All the endpoints for the Samizdat HTTP API.
fn api() -> Router {
    Router::new()
        .nest("/connected-ips", connected_ips())
        .nest("/resolution-order", resolution_order())
        .nest("/blacklisted-ips", blacklisted_ips::api())
        .nest("/connection-logs", connection_logs())
        .nest("/candidate-logs", candidate_logs())
        .nest("/statistics-logs", statistics_logs())
        .nest("/query-logs", query_logs())
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

/// Gets the connection information for a range of IDs.
fn connection_logs() -> Router {
    #[serde_inline_default]
    #[derive(Deserialize)]
    struct QueryParameters {
        #[serde_inline_default(Id::MIN)]
        start: Id,
        #[serde_inline_default(Id::MAX)]
        end: Id,
        #[serde_inline_default(usize::MAX)]
        limit: usize,
    }

    #[derive(Serialize)]
    struct ConnectionLogResponse {
        logs: Vec<ConnectionLog>,
    }

    Router::new().route(
        "/",
        get(
            |Query(QueryParameters { start, end, limit }): Query<QueryParameters>| {
                tokio::task::spawn_blocking(move || {
                    readonly_tx(|tx| {
                        let mut logs = vec![];

                        ConnectionLog::range(start, end).for_each(tx, |_, serialized| {
                            if logs.len() >= limit {
                                return Ok(Some(()));
                            }

                            logs.push(bincode::deserialize(serialized)?);

                            Ok(None)
                        })?;

                        Ok(ConnectionLogResponse { logs })
                    })
                })
                .map(|outcome| outcome.expect("blocking task panicked"))
                .map(ApiResponse)
            },
        ),
    )
}

/// Gets the query logs for a range of IDs.
fn query_logs() -> Router {
    #[serde_inline_default]
    #[derive(Deserialize)]
    struct QueryParameters {
        #[serde_inline_default(Id::MIN)]
        start: Id,
        #[serde_inline_default(Id::MAX)]
        end: Id,
        #[serde_inline_default(usize::MAX)]
        limit: usize,
    }

    #[derive(Serialize)]
    struct QueryLogsResponse {
        logs: Vec<QueryLog>,
    }

    Router::new().route(
        "/",
        get(
            |Query(QueryParameters { start, end, limit }): Query<QueryParameters>| {
                tokio::task::spawn_blocking(move || {
                    readonly_tx(|tx| {
                        let mut logs = vec![];

                        QueryLog::range(start, end).for_each(tx, |_, serialized| {
                            if logs.len() >= limit {
                                return Ok(Some(()));
                            }

                            logs.push(bincode::deserialize(serialized)?);
                            Ok(None)
                        })?;

                        Ok(QueryLogsResponse { logs })
                    })
                })
                .map(|outcome| outcome.expect("blocking task panicked"))
                .map(ApiResponse)
            },
        ),
    )
}

/// Gets the candidate logs for a range of IDs.
fn candidate_logs() -> Router {
    #[serde_inline_default]
    #[derive(Deserialize)]
    struct QueryParameters {
        #[serde_inline_default(Id::MIN)]
        start: Id,
        #[serde_inline_default(Id::MAX)]
        end: Id,
        #[serde_inline_default(usize::MAX)]
        limit: usize,
    }

    #[derive(Serialize)]
    struct CandidateLogsResponse {
        logs: Vec<CandidateLog>,
    }

    Router::new().route(
        "/",
        get(
            |Query(QueryParameters { start, end, limit }): Query<QueryParameters>| {
                tokio::task::spawn_blocking(move || {
                    readonly_tx(|tx| {
                        let mut logs = vec![];

                        CandidateLog::range(start, end).for_each(tx, |_, serialized| {
                            if logs.len() >= limit {
                                return Ok(Some(()));
                            }

                            logs.push(bincode::deserialize(serialized)?);
                            Ok(None)
                        })?;

                        Ok(CandidateLogsResponse { logs })
                    })
                })
                .map(|outcome| outcome.expect("blocking task panicked"))
                .map(ApiResponse)
            },
        ),
    )
}

/// Gets the statistic logs for a range of IDs.
fn statistics_logs() -> Router {
    #[serde_inline_default]
    #[derive(Deserialize)]
    struct QueryParameters {
        #[serde_inline_default(Id::MIN)]
        start: Id,
        #[serde_inline_default(Id::MAX)]
        end: Id,
        #[serde_inline_default(usize::MAX)]
        limit: usize,
        #[serde(default)]
        statistics_type: Option<StatisticsType>,
        #[serde(default)]
        peers: Vec<IpAddr>,
    }

    #[derive(Serialize)]
    struct StatisticsLogsResponse {
        logs: Vec<StatisticsLog>,
    }

    Router::new().route(
        "/",
        get(
            |Query(QueryParameters {
                 start,
                 end,
                 limit,
                 statistics_type,
                 peers,
             }): Query<QueryParameters>| {
                tokio::task::spawn_blocking(move || {
                    let peers = peers.into_iter().collect::<BTreeSet<_>>();
                    readonly_tx(|tx| {
                        let mut logs = vec![];

                        StatisticsLog::range(start, end).for_each(tx, |_, serialized| {
                            if logs.len() >= limit {
                                return Ok(Some(()));
                            }

                            let log: StatisticsLog = bincode::deserialize(serialized)?;

                            if let Some(statistics_type) = statistics_type {
                                if statistics_type != log.statistics().statistics_type {
                                    return Ok(None);
                                }
                            }

                            if !peers.is_empty() && !peers.contains(&log.statistics().peer_ip) {
                                return Ok(None);
                            }

                            logs.push(log);
                            Ok(None)
                        })?;

                        Ok(StatisticsLogsResponse { logs })
                    })
                })
                .map(|outcome| outcome.expect("blocking task panicked"))
                .map(ApiResponse)
            },
        ),
    )
}
