//! Access token and port management for the Samizdat HTTP API.

use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::cli::cli;

/// Access token used for authenticating requests to the Samizdat HTTP API.
///
/// The token is loaded from a file in the local filesystem and cached in memory. Its
/// main purpose is to _only allow_ applications that have access to the local
/// filesystem to be able to access the node.
static ACCESS_TOKEN: OnceLock<String> = OnceLock::new();

/// Retrieves the access token. Must be called after initialization.
pub fn access_token<'a>() -> Result<&'a str, anyhow::Error> {
    Ok(ACCESS_TOKEN.get_or_try_init(init_access_token)?.as_str())
}

/// Initializes access token. The access token is a file in the local
/// filesystem that grants access to protected routes in the Samizdat HTTP API.
fn init_access_token() -> Result<String, anyhow::Error> {
    let path = data_file("access-token");
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "cannot read access token at {}. Is samizdat-node running with --data={}?",
            path.display(),
            cli().data.display()
        )
    })?;
    Ok(contents.trim().to_owned())
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
