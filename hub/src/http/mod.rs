mod auth;

use futures::Future;
use warp::Filter;

use crate::rpc::ROOM;
use crate::{balanced_or_tree, CLI};

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
        //_ => http::StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn api_reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: serde::Serialize,
{
    let status = t
        .as_ref()
        .map_err(error_status_code)
        .err()
        .unwrap_or_default();
    let json = t.map_err(|err| err.to_string());
    warp::reply::with_header(
        warp::reply::with_status(
            serde_json::to_string_pretty(&json).expect("can serialize JSON"),
            status,
        ),
        http::header::CONTENT_TYPE,
        "application/json",
    )
}

/// Utility to create a tuple of one value _very explicitly_.
fn tuple<T>(t: T) -> (T,) {
    (t,)
}

pub fn serve() -> impl Future<Output = ()> {
    let server = warp::filters::addr::remote()
        .and_then(|addr: Option<std::net::SocketAddr>| async move {
            if let Some(addr) = addr {
                if addr.ip().to_canonical().is_loopback() {
                    return Err(warp::reject::not_found());
                }
            }

            Ok(warp::reply::with_status(
                "cannot connect outside loopback",
                ::http::StatusCode::FORBIDDEN,
            ))
        })
        .or(warp::get().and(warp::path::end()).map(|| {
            warp::reply::with_header(include_str!("../index.html"), "Content-Type", "text/html")
        }))
        .or(self::api())
        .with(warp::log("api"));

    // Run public server:
    warp::serve(server).run(([0; 16], CLI.http_port))
}

fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(connected())
}

fn connected() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path("connected-ips")
        .and(warp::get())
        .and_then(|| async {
            let ips = ROOM
                .raw_participants()
                .await
                .iter()
                .map(|(addr, _)| *addr)
                .collect::<Vec<_>>();
            Ok(api_reply(Ok(ips))) as Result<_, warp::Rejection>
        })
        .map(tuple)
}
