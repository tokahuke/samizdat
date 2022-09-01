//! HTTP API for the Samizdat Node.

mod auth;
mod collections;
mod editions;
mod identities;
mod kvstore;
mod objects;
mod redirects;
mod resolvers;
mod series;
mod subscriptions;

pub use auth::authenticate;

use futures::Future;
use warp::Filter;

use crate::{balanced_or_tree, cli};

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

fn html(rendered: String) -> impl warp::Reply {
    warp::reply::with_header(
        warp::reply::with_status(rendered, http::StatusCode::OK),
        http::header::CONTENT_TYPE,
        "text/html; charset=UTF-8",
    )
}

/// The entrypoint of the Samizdat node public HTTP API.
fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        kvstore::api(),                // kvstore not subject to redirect rules.
        redirects::general_redirect(), // redirect rules here...
        objects::api(),
        collections::api(),
        series::api(),
        editions::api(),
        identities::api(),
        subscriptions::api(),
        auth::api(),
        post_vacuum(),
    )
    .recover(|rejection: warp::Rejection| async move {
        if let Some(forbidden) = rejection.find::<auth::Forbidden>() {
            Ok(warp::reply::with_status(
                forbidden.to_string(),
                http::StatusCode::FORBIDDEN,
            ))
        } else if let Some(unauthorized) = rejection.find::<auth::Unauthorized>() {
            Ok(warp::reply::with_status(
                unauthorized.to_string(),
                http::StatusCode::UNAUTHORIZED,
            ))
        } else if let Some(error) = rejection.find::<crate::Error>() {
            Ok(warp::reply::with_status(
                error.to_string(),
                http::StatusCode::BAD_REQUEST,
            ))
        } else {
            Err(rejection)
        }
    })
}

/// Triggers a manual vacuum round.
fn post_vacuum() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::post()
        .and(warp::path!("_vacuum"))
        .map(|| crate::vacuum::vacuum())
        .map(api_reply)
}

pub fn serve() -> impl Future<Output = ()> {
    let public_server = warp::filters::addr::remote()
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
    warp::serve(public_server).run(([0; 16], cli().port))
}
