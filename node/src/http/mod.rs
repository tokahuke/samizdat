//! HTTP API for the Samizdat Node.

mod auth;
mod collections;
mod identities;
mod kvstore;
mod objects;
mod resolvers;
mod series;
mod subscriptions;

pub use auth::authenticate;

use warp::Filter;

use crate::balanced_or_tree;

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
        crate::Error::DifferentePublicKeys => http::StatusCode::BAD_REQUEST,
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

/// Utility to create a tuple of one value _very explicitely_.
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

/// Optionaly implements the "tilde redirect". Similarly to Unix platforms, the `~`
/// represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let entity_type = split.next()?;
    let entity_identifier = split.next()?;

    let mut found_tilde = false;
    for item in &mut split {
        if item == "~" {
            found_tilde = true;
            break;
        }
    }

    if found_tilde {
        let tail = split.collect::<Vec<_>>().join("/");
        Some(format!("/{}/{}/{}", entity_type, entity_identifier, tail))
    } else {
        None
    }
}

/// Optionally redirects a "home path" without trailing slash to the same path with
/// trailing slash.
fn maybe_redirect_base(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let entity_type = split.next()?;
    let entity_identifier = split.next()?;
    let is_redirectable_entity = entity_type == "_collections" || entity_type == "_series";

    if split.next().is_none() && is_redirectable_entity {
        Some(format!("/{}/{}/", entity_type, entity_identifier))
    } else {
        None
    }
}

/// Removes empty path segments from the URL.
fn maybe_redirect_empty(path: &str) -> Option<String> {
    if path.contains("//") {
        let split = path.split('/');
        let without_initial_slash = split
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        Some(format!("/{}", without_initial_slash))
    } else {
        None
    }
}

/// Does all the redirection dances and shenenigans.
pub fn general_redirect(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::get()
        .and(warp::path::tail())
        .and_then(|path: warp::path::Tail| async move {
            let maybe_redirect = maybe_redirect_tilde(path.as_str())
                .or_else(|| maybe_redirect_base(path.as_str()))
                .or_else(|| maybe_redirect_empty(path.as_str()));

            if let Some(location) = maybe_redirect {
                log::info!("location {}", location);
                let uri = location
                    .parse::<http::uri::Uri>()
                    .expect("bad route on tilde redirect");
                Ok(warp::redirect(uri))
            } else {
                Err(warp::reject::reject())
            }
        })
}

/// The entrypoint of the Samizdat node public HTTP API.
pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        kvstore::api(),     // kvstore not subject to redirect rules.
        general_redirect(), // redirect rules here...
        objects::api(),
        collections::api(),
        series::api(),
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
pub fn post_vacuum() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::post()
        .and(warp::path!("_vacuum"))
        .map(|| crate::vacuum::vacuum())
        .map(api_reply)
}
