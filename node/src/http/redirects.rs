//! Redirects for the HTTP API.

use std::borrow::Cow;

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

/// Optionally implements the "tilde redirect". Similarly to Unix platforms, the `~`
/// represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde(path: &str) -> Option<String> {
    let mut split = path.split('/');
    assert_eq!(split.next()?, ""); // path always starts with '/'!

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

    if !found_tilde {
        return None;
    }

    let tail = after_tilde.join("/");

    tracing::debug!("applied maybe_redirect_tilde");
    Some(format!("/{entity_type}/{entity_identifier}/{tail}"))
}

/// Optionally implements the "tilde redirect" for identities. Similarly to Unix platforms,
/// the `~` represents the "home folder" of a collection or a series.
fn maybe_redirect_tilde_identity(path: &str) -> Option<String> {
    let mut split = path.split('/');
    assert_eq!(split.next()?, ""); // path always starts with '/'!

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

    if !found_tilde {
        return None;
    }

    tracing::debug!("applied maybe_redirect_tilde_identity");
    let tail = after_tilde.join("/");

    Some(format!("/{identity}/{tail}"))
}

/// Removes empty path segments from the URL.
fn maybe_redirect_empty(path: &str) -> Option<String> {
    if !path.contains("//") {
        return None;
    }

    let split = path.split('/');

    let without_double_slash = split
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");

    tracing::debug!("applied maybe_redirect_empty");
    Some(format!("/{without_double_slash}"))
}

/// All redirects together.
fn maybe_redirect(path: &str) -> Option<String> {
    maybe_redirect_tilde(path)
        .or_else(|| maybe_redirect_tilde_identity(path))
        .or_else(|| maybe_redirect_empty(path))
}

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
        return Redirect::permanent(&path).into_response();
    }

    next.run(request).await
}
