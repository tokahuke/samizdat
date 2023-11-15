//! Identities API.

use std::fmt::Display;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use warp::path::Tail;
use warp::Filter;

use crate::db::Table;
use crate::http::api_reply;
use crate::identity_dapp::identity_provider;
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
                    .unwrap_or_else(|| {
                        samizdat_common::blockchain::DEFAULT_PROVIDER_ENDPOINT.to_owned()
                    }),
            })
        })
        .map(api_reply)
}

/// A reference to an identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IdentityRef {
    /// A valid identity handle.
    handle: String,
}

impl FromStr for IdentityRef {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            invalid @ ("" | "~" | "." | "..") => {
                Err(format!("Identity handle cannot be `{invalid}`").into())
            }
            s if s.starts_with('_') => {
                Err(format!("Identity handle `{s}` starting with `_`").into())
            }
            s => Ok(IdentityRef {
                handle: s.to_owned(),
            }),
        }
    }
}

impl Display for IdentityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.handle)
    }
}

impl IdentityRef {
    /// Gets the handle (i.e., human-readable name) of this identity.
    pub fn handle(&self) -> &str {
        &self.handle
    }
}

/// Gets the contents of an item using identity.
fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!(IdentityRef / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|identity: IdentityRef, name: Tail| async move {
            Ok(resolve_identity(identity.handle(), name.as_str().into(), []).await?)
                as Result<_, warp::Rejection>
        })
        .map(tuple)
}
