//! Access rights infrastructure for the node.
//!
//! This module implements two complementary access control systems:
//!
//! 1. Access tokens: A filesystem-based authentication system for local
//!    applications. The node maintains TWO tokens with different scopes,
//!    so day-to-day introspection does not require root:
//!
//!      - `read-token`  (mode 0644): grants read-only access to the
//!         admin API (list series / subscriptions / hubs / connections,
//!         look up auths, etc.). World-readable so any local shell can
//!         use the CLI without sudo.
//!      - `admin-token` (mode 0600): grants the full admin scope,
//!         including state-mutating routes (commit, import, series new,
//!         subscription new/rm, hub new/rm, identity ops). Owner-only.
//!
//!    The token a request presents in `Authorization: Bearer <t>` is
//!    classified by the auth middleware (see `http/auth.rs`); routes
//!    declare the minimum [`TokenScope`] they accept.
//!
//! 2. Access rights: A permission system for web applications running in browsers. It defines
//!    different levels of access (from public access to management capabilities) that can be
//!    granted to web-based clients, ensuring fine-grained control over API operations.

use serde_derive::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt::{self, Display};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::OnceLock;

use samizdat_common::Hash;

use crate::cli;

/// What a bearer token is allowed to do. Strictly ordered: `Admin`
/// is a superset of `Read`. Anywhere the middleware needs "at least"
/// a scope, use `>=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TokenScope {
    /// Can read state-introspecting endpoints. Cannot mutate anything,
    /// cannot sign anything, cannot view secrets (e.g. series private
    /// key bytes are still scrubbed in responses).
    Read,
    /// Full admin: every protected route, including state-mutating
    /// ones (commit, import, series new, hub/subscription/identity
    /// mutations).
    Admin,
}

/// In-memory copy of both tokens, set at startup by [`init_access_token`].
static READ_TOKEN: OnceLock<String> = OnceLock::new();
static ADMIN_TOKEN: OnceLock<String> = OnceLock::new();

/// Retrieves the admin token. Must be called after initialization.
pub fn admin_token<'a>() -> &'a str {
    ADMIN_TOKEN.get().expect("admin token not initialized")
}

/// Retrieves the read-only token. Must be called after initialization.
pub fn read_token<'a>() -> &'a str {
    READ_TOKEN.get().expect("read token not initialized")
}

/// Generates a new token value.
fn gen_token() -> String {
    Hash::rand().to_string()
}

/// Reads an existing token file, or creates one with a fresh value at
/// the given Unix mode. Returns the token string.
fn read_or_create_token(path: &str, #[cfg(unix)] mode: u32) -> Result<String, crate::Error> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(mode);
    match opts.open(path) {
        Ok(mut file) => {
            let token = gen_token();
            file.write_all(token.as_bytes())?;
            Ok(token)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Ok(fs::read_to_string(path)?.trim().to_owned())
        }
        Err(error) => Err(error.into()),
    }
}

/// Initialise both access tokens (read + admin). See module docs for the
/// scope split and the file-permission rationale.
pub fn init_access_token() -> Result<(), crate::Error> {
    let data: PathBuf = cli().data.clone();

    let admin_path = data.join("admin-token");
    // 0640 (owner rw, group r) instead of 0600: admin scope is shared
    // among members of the daemon's group (typically `samizdat`),
    // managed at the OS level by `samizdat-up admin add/rm`. The
    // setgid bit on the data dir, set by samizdat-up at install
    // time, ensures the file's group matches the dir's group so the
    // group-read bit is actually meaningful.
    let admin = read_or_create_token(
        admin_path
            .to_str()
            .expect("admin-token path is not a string"),
        #[cfg(unix)]
        0o640,
    )?;
    let read_path = data.join("read-token");
    let read = read_or_create_token(
        read_path.to_str().expect("read-token path is not a string"),
        #[cfg(unix)]
        0o644,
    )?;

    // Deliberately do NOT log the token bodies: they grant API access
    // to the node and would leak via journald, file logging, or shared
    // shells. Lengths only.
    tracing::info!(
        "Node tokens initialised (read length {}, admin length {})",
        read.len(),
        admin.len()
    );
    READ_TOKEN.set(read).ok();
    ADMIN_TOKEN.set(admin).ok();

    // ... and also piggyback writing port here. I know this is hacky, but...
    let port_path = data.join("port");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&port_path)?;
    file.write_all(cli().port.to_string().as_bytes())?;

    Ok(())
}

/// Represents the access rights that can be granted to web applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum AccessRight {
    /// Can access public content. This is granted by default to everyone.
    Public,
    /// Can create and delete objects.
    ManageObjects,
    /// Can get statistics on object usage.
    GetObjectStats,
    /// Can create and delete bookmarks.
    ManageBookmarks,
    /// Can create collections.
    ManageCollections,
    /// Can create and delete series (including private keys).
    ManageSeries,
    /// Can create and delete subscriptions.
    ManageSubscriptions,
    /// Can create and delete identities.
    ManageIdentities,
    /// Can create and delete connection to Samizdat Hubs.
    ManageHubs,
}

impl PartialOrd for AccessRight {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(match (self, other) {
            (this, that) if *this as u8 == *that as u8 => Ordering::Equal,
            (Self::Public, _) => Ordering::Less,
            _ => return None,
        })
    }
}

/// A name of an entity inside the Samizdat network.
///
/// An entity can be an object, a collection item, a series item, etc...
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    /// The type of the entity.
    r#type: String,
    /// The identifier of the entity.
    identifier: String,
}

impl Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.r#type, self.identifier)
    }
}

impl Entity {
    /// Creates an Entity from a URL path string.
    /// Returns None if the path cannot be parsed into a valid entity.
    pub fn from_path(path: &str) -> Option<Entity> {
        let mut split = path.split('/');
        let mut r#type = split.next()?;

        if r#type.is_empty() {
            r#type = split.next()?;
        }

        if r#type.starts_with('_') {
            // Non-identity based access.
            let identifier = split.next()?;

            Some(Entity {
                r#type: r#type.to_owned(),
                identifier: identifier.to_owned(),
            })
        } else {
            Some(Entity {
                r#type: "_identity".to_owned(),
                identifier: r#type.to_owned(),
            })
        }
    }
}
