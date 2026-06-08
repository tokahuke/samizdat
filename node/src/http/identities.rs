//! Identity handle parser.
//!
//! Identity content is served at `http://<identity>.localhost:<port>/<path>`
//! by the dispatcher in `node/src/http/content.rs`. This module retains the
//! [`IdentityRef`] handle parser used by callers that need to validate a
//! handle string.

use std::fmt::Display;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};

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
    #[allow(dead_code)]
    pub fn handle(&self) -> &str {
        &self.handle
    }
}
