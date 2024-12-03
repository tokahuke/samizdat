//! Redirects for the HTTP API.

use std::borrow::Cow;

use axum::{extract::Request, middleware::Next, response::Response};

/// Optionally implements the "tilde redirect". Similarly to Unix platforms, the `~`
/// represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let entity_type = split.next()?;
    let entity_identifier = split.next()?;

    // It's an identity, not an entity. Other rules apply.
    if !entity_type.starts_with('_') {
        return None;
    }

    // Find the last tilde and everything after it.
    let mut found_tilde = false;
    let mut after_tilde = vec![];

    for item in split {
        if item == "~" {
            found_tilde = true;
            after_tilde.clear();
        } else {
            after_tilde.push(item);
        }
    }

    if found_tilde {
        let tail = after_tilde.join("/");
        Some(format!("/{entity_type}/{entity_identifier}/{tail}"))
    } else {
        None
    }
}

/// Optionally implements the "tilde redirect" for identities. Similarly to Unix platforms,
/// the `~` represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde_identity(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let identity = split.next()?;

    // It's an entity, not an identity. Other rules apply.
    if identity.starts_with('_') {
        return None;
    }

    // Find the last tilde and everything after it.
    let mut found_tilde = false;
    let mut after_tilde = vec![];

    for item in split {
        if item == "~" {
            found_tilde = true;
            after_tilde.clear();
        } else {
            after_tilde.push(item);
        }
    }

    if found_tilde {
        let tail = after_tilde.join("/");
        Some(format!("/{identity}/{tail}"))
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
        Some(format!("/{entity_type}/{entity_identifier}/"))
    } else {
        None
    }
}

/// Optionally redirects a "home path" for identities without trailing slash to the same path with
/// trailing slash.
fn maybe_redirect_base_identity(path: &str) -> Option<String> {
    let mut split = path.split('/');
    let identity = split.next()?;

    // It's an entity, not an identity. Other rules apply.
    if identity.starts_with('_') {
        return None;
    }

    if split.next().is_none() {
        Some(format!("/{identity}/"))
    } else {
        None
    }
}

/// Removes empty path segments from the URL.
fn maybe_redirect_empty(path: &str) -> Option<String> {
    if path.contains("//") {
        let split = path.split('/');
        let without_double_slash = split
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        Some(format!("/{without_double_slash}"))
    } else {
        None
    }
}

/// All redirects together.
fn maybe_redirect(path: &str) -> Option<String> {
    maybe_redirect_tilde(path)
        .or_else(|| maybe_redirect_tilde_identity(path))
        .or_else(|| maybe_redirect_base(path))
        .or_else(|| maybe_redirect_base_identity(path))
        .or_else(|| maybe_redirect_empty(path))
}

// pub fn general_redirect(
// ) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
//     warp::get()
//         .and(warp::path::tail())
//         .and_then(|initial_path: warp::path::Tail| async move {
//             let mut path = initial_path.as_str().to_owned();
//             let mut was_redirected = false;

//             while let Some(new_path) = maybe_redirect(&path) {
//                 path = new_path;
//                 was_redirected = true;
//             }

//             if was_redirected {
//                 log::info!("location {}", path);
//                 let uri = path
//                     .parse::<http::uri::Uri>()
//                     .expect("bad route on redirect");
//                 Ok(warp::redirect(uri))
//             } else {
//                 Err(warp::reject::reject())
//             }
//         })
// }

/// Does all the redirection dances and shenanigans.
pub async fn redirect_request(request: Request, next: Next) -> Response {
    // Exceptions:
    if request.uri().path().starts_with("/_kvstore/") {
        return next.run(request).await;
    }

    let mut path = Cow::Borrowed(request.uri().path());
    let mut was_redirected = false;

    while let Some(new_path) = maybe_redirect(&path) {
        path = Cow::Owned(new_path);
        was_redirected = true;
    }

    if was_redirected {
        return Response::builder()
            .status(http::StatusCode::PERMANENT_REDIRECT)
            .header("Location", path.as_ref())
            .body("308 Permanent Redirect".into())
            .expect("can create redirect response");
    }

    next.run(request).await
}
