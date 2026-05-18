//! Resolving + downloading binaries from the `get-samizdat`
//! content-addressed collection.
//!
//! The proxy serves the collection at:
//!
//!     https://proxy.hubfederation.com/~get-samizdat/<version>/<target-triple>/<component>/<file>
//!
//! `latest/` works as the version label and points at the most recent
//! signed edition.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::time::Duration;

/// Default origin for the published get-samizdat collection. Override
/// at install-time with `--from <URL>` (used by the integration test
/// workflow with a `file://` path pointing at locally-built artifacts).
pub const DEFAULT_ORIGIN: &str = "https://proxy.hubfederation.com/~get-samizdat";

/// What triple to fetch for. Defaults to whatever the running
/// samizdat-up was compiled for; the result is stable across the
/// process and matches the binaries we know how to run.
pub fn host_target_triple() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-gnu"
    } else {
        // Compile-time: any unsupported target produces a string the
        // server is guaranteed to 404 on, surfacing the issue loudly
        // rather than silently mis-fetching.
        "UNSUPPORTED-TARGET-TRIPLE"
    }
}

/// One file as fetched from the collection: the bytes + the source URL,
/// for error context.
pub struct Fetched {
    pub bytes: Vec<u8>,
    pub source: String,
}

/// Download a file from the collection. `origin` is the URL prefix up
/// to but not including `<version>`, normally [`DEFAULT_ORIGIN`].
pub fn fetch_file(
    origin: &str,
    version: &str,
    target_triple: &str,
    component: &str,
    file: &str,
) -> Result<Fetched> {
    let url = format!("{origin}/{version}/{target_triple}/{component}/{file}");

    if let Some(local_path) = strip_file_scheme(&url) {
        // file://... -- local artifact path, used by the integration
        // test workflow so we exercise install logic without depending
        // on the testbed being up.
        let bytes = std::fs::read(&local_path)
            .with_context(|| format!("reading local artifact at {}", local_path.display()))?;
        return Ok(Fetched {
            bytes,
            source: url,
        });
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("building http client")?;

    let resp = client
        .get(&url)
        .send()
        .with_context(|| format!("requesting {url}"))?;

    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url} returned HTTP {status}");
    }

    let bytes = resp
        .bytes()
        .with_context(|| format!("reading body of {url}"))?
        .to_vec();
    Ok(Fetched { bytes, source: url })
}

fn strip_file_scheme(url: &str) -> Option<PathBuf> {
    url.strip_prefix("file://").map(PathBuf::from)
}

/// `samizdat-up versions [--remote]`. The local listing is the union of
/// directories under each component's install root; the remote listing
/// queries the collection's `_inventory` for known versions.
pub fn list_versions(remote: bool) -> Result<()> {
    if remote {
        // TODO(samizdat-up v2): query the collection's `_inventory`
        // edition manifest to enumerate versions. For now print the
        // public-resolution endpoint so the user can see what is
        // available with their own browser.
        println!("Remote version listing not yet implemented.");
        println!("Available editions can be browsed at:");
        println!("  {DEFAULT_ORIGIN}/");
        return Ok(());
    }
    println!("Local version listing not yet implemented.");
    println!("(once `samizdat-up install` writes a versions manifest,");
    println!(" this will summarise what is on the box)");
    Ok(())
}
