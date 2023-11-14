//! Identities API.

use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use warp::path::Tail;
use warp::Filter;

use crate::db::Table;
use crate::http::api_reply;
use crate::identity_dapp::{identity_provider, DEFAULT_PROVIDER_ENDPOINT};
use crate::{balanced_or_tree, db};

use super::resolvers::resolve_identity;
use super::tuple;

/// The entrypoint of the object API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // Query item using identity
        get_item(),
        get_ethereum_provider(),
        put_ethereum_provider(),
    )
}

/// Sets the Ethereum Network Provider to be used.
fn put_ethereum_provider(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[serde_as]
    #[derive(Deserialize)]
    struct Request {
        endpoint: String,
    }

    #[derive(Serialize)]
    struct Response {}

    warp::path!("_ethereum_provider")
        .and(warp::put())
        .and(warp::body::json())
        .map(|request: Request| {
            tokio::spawn(async move { identity_provider().set_endpoint(&request.endpoint).await });
            Ok(Response {})
        })
        .map(api_reply)
}

/// Gets the Ethereum Network Provider to be used.
fn get_ethereum_provider(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Serialize)]
    struct Response {
        endpoint: String,
    }

    warp::path!("_ethereum_provider")
        .and(warp::get())
        .map(|| {
            Ok(Response {
                endpoint: db()
                    .get_cf(Table::Global.get(), "ethereum_provider_endpoint")?
                    .map(|e| String::from_utf8_lossy(&e).into_owned())
                    .unwrap_or_else(|| DEFAULT_PROVIDER_ENDPOINT.to_owned()),
            })
        })
        .map(api_reply)
}

/// Gets the contents of an item using identity.
fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!(String / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|identity: String, name: Tail| async move {
            Ok(resolve_identity(&identity, name.as_str().into(), []).await?)
                as Result<_, warp::Rejection>
        })
        .map(tuple)
}
