//! HTTP API for the Samizdat Node.

mod collections;
mod objects;
mod resolvers;
mod returnable;
mod series;

pub use returnable::{Json, Return, Returnable};

use warp::Filter;

use crate::balanced_or_tree;

/// Transforms a `Result<T, crate::Error>` into a Warp reply.
fn reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: Returnable,
{
    warp::reply::with_header(
        warp::reply::with_status(t.render().into_owned(), t.status_code()),
        http::header::CONTENT_TYPE,
        &*t.content_type(),
    )
}

/// Transforms a `Result<T, crate::Error>` future into a Warp reply.
async fn async_reply<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

/// Utility to create a tuple of one value _very explicitely_.
fn tuple<T>(t: T) -> (T,) {
    (t,)
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

/// The entrypoint of the Samizdat node HTTP API.
pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        general_redirect(),
        objects::api(),
        collections::api(),
        series::api(),
    )
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
