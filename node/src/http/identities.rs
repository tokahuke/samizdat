//! Identities API.

use std::fmt::Display;
use std::str::FromStr;

use axum::extract::Path;
use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use futures::FutureExt;
use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use tokio::time::Instant;

use crate::access::AccessRight;
use crate::http::{PageResponse, SamizdatTimeout};
use crate::security_scope;

use super::resolvers::resolve_identity;

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
pub fn api() -> Router {
    #[serde_as]
    #[derive(Deserialize)]
    struct IdentityPath {
        #[serde_as(as = "DisplayFromStr")]
        identity: IdentityRef,
        name: String,
    }

    Router::new()
        .route(
            "/~:identity/*name",
            get(
                |Path(IdentityPath { identity, name }): Path<IdentityPath>,
                 SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        resolve_identity(
                            identity.handle(),
                            name.as_str().into(),
                            [],
                            Instant::now() + timeout,
                        )
                        .await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            "/~:identity/",
            get(
                |Path(identity): Path<String>, SamizdatTimeout(timeout): SamizdatTimeout| {
                    async move {
                        resolve_identity(&identity, "".into(), [], Instant::now() + timeout).await
                    }
                    .map(PageResponse)
                },
            )
            .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            "/~:identity",
            get(|Path(identity): Path<String>| async move {
                Redirect::permanent(&format!("{identity}/"))
            }),
        )
}
