//! Path-level redirects. Currently only the double-slash compaction.

use std::borrow::Cow;

use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};

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

/// Does all the redirection dances and shenanigans.
pub async fn redirect_request(request: Request, next: Next) -> Response {
    let mut path = Cow::Borrowed(request.uri().path());
    let mut was_redirected = false;

    while let Some(new_path) = maybe_redirect_empty(&path) {
        path = Cow::Owned(new_path);
        was_redirected = true;
    }

    if was_redirected {
        return Redirect::permanent(&path).into_response();
    }

    next.run(request).await
}
