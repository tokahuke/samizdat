//! Access token and port management for the Samizdat HTTP API.
//!
//! The node maintains two tokens with different scopes (see
//! `node/src/access.rs`): `admin-token` (mode 0600, full scope) and
//! `read-token` (mode 0644, read-only scope). The CLI picks the
//! highest-scope token it can read: admin if accessible (typically
//! under sudo or as a member of the data-dir group), otherwise read.
//! Commands that need admin scope but were invoked with only the
//! read token get a clear 403 from the node ("try sudo"); we do not
//! gate per-subcommand in the CLI, so future scope changes only
//! require a server-side edit.

use anyhow::{Context, anyhow};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;


use crate::cli::cli;

/// Highest-scope token the CLI was able to read from the data dir.
/// Loaded lazily, cached for the rest of the process.
static ACCESS_TOKEN: OnceLock<String> = OnceLock::new();

/// Retrieves the highest-scope access token the CLI can read. Must be
/// called after initialization.
pub fn access_token<'a>() -> Result<&'a str, anyhow::Error> {
    Ok(ACCESS_TOKEN.get_or_try_init(init_access_token)?.as_str())
}

/// Resolve a usable access token. Tries `admin-token` first; falls back
/// to `read-token` if admin-token is not readable by this process (the
/// common no-sudo case). The node writes both at startup.
fn init_access_token() -> Result<String, anyhow::Error> {
    let admin = data_file("admin-token");
    match fs::read_to_string(&admin) {
        Ok(contents) => return Ok(contents.trim().to_owned()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // Expected for an unprivileged shell; fall through to read-token.
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Fall through to read-token. The node always writes both,
            // so NotFound here means the data dir is wrong or the node
            // never came up; either way `read-token` will fail the
            // same way and the error below carries both paths.
        }
        Err(error) => {
            return Err(anyhow!(error))
                .with_context(|| format!("reading admin-token at {}", admin.display()));
        }
    }

    let read = data_file("read-token");
    fs::read_to_string(&read)
        .map(|s| s.trim().to_owned())
        .with_context(|| {
            format!(
                "cannot read admin-token ({}) or read-token ({}). \
                 Is samizdat-node running with --data={}? \
                 If yes, retry with `sudo` for admin operations.",
                admin.display(),
                read.display(),
                cli().data.display(),
            )
        })
}

/// Port number used by the Samizdat HTTP API. The port is loaded from a file in the local
/// filesystem and cached in memory.
static PORT: OnceLock<u16> = OnceLock::new();

/// Retrieves the HTTP server port. Must be called after initialization.
pub fn port() -> Result<u16, anyhow::Error> {
    Ok(*PORT.get_or_try_init(init_port)?)
}

/// Initializes HTTP port value.
pub fn init_port() -> Result<u16, anyhow::Error> {
    let path = data_file("port");
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "cannot read port file at {}. Is samizdat-node running with --data={}?",
            path.display(),
            cli().data.display()
        )
    })?;
    contents
        .trim()
        .parse::<u16>()
        .with_context(|| format!("port file at {} does not contain a valid u16", path.display()))
}

/// Build a path inside the configured data dir. Uses `Path::join` so the
/// result is a real `PathBuf` (no non-UTF8 panic) and so non-ASCII data
/// dirs work as well as ASCII ones.
fn data_file(name: &str) -> PathBuf {
    let mut p: PathBuf = cli().data.clone();
    p.push(Path::new(name));
    p
}
