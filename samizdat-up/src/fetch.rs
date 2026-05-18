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
use std::process::Command;
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
///
/// Transit integrity comes from HTTPS (TLS MAC on the body). End-to-
/// end signature verification is not done here: the proxy stamps an
/// `X-Samizdat-Object` header with `Sha3_224(body)`, but that hash is
/// computed by whoever served the bytes, so it cannot defend against
/// a compromised proxy serving different bytes. The real defense is
/// the `docs/deferred.md` "V2 trust model" item: fetch the signed
/// inventory + verify objects against the series public key baked
/// into samizdat-up.
pub fn fetch_file(
    origin: &str,
    version: &str,
    target_triple: &str,
    component: &str,
    file: &str,
) -> Result<Fetched> {
    let url = format!("{origin}/{version}/{target_triple}/{component}/{file}");

    if let Some(local_path) = strip_file_scheme(&url) {
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

/// `samizdat-up versions [--remote]`.
///
/// Local listing: print our own version, then run `--version` on each
/// Samizdat binary we find under the platform's install paths. There
/// is no separate "versions manifest" on disk; the binaries themselves
/// are the source of truth (clap's `--version` reports the version
/// baked in at build time).
///
/// Remote listing: fetch `<origin>/latest/install.sh` and parse the
/// version from its header comment. The collection does not expose a
/// directory-listing API today, so we surface "latest" rather than an
/// enumeration of all published editions.
pub fn list_versions(remote: bool) -> Result<()> {
    println!("samizdat-up {} (this binary)", env!("CARGO_PKG_VERSION"));

    let installed = crate::install::installed_binary_paths();
    if installed.is_empty() {
        println!("(no Samizdat binaries found in standard install paths)");
    } else {
        for (name, path) in &installed {
            let version = query_version(path).unwrap_or_else(|_| "unknown".to_owned());
            // clap reports "<bin-name> X.Y.Z"; the binary name is
            // already in the first column, so collapse to just the
            // version token for readability.
            let version = version
                .split_whitespace()
                .nth(1)
                .unwrap_or(&version)
                .to_owned();
            println!("{name:<14} {version} (at {})", path.display());
        }
    }

    if remote {
        println!();
        match latest_remote_version(DEFAULT_ORIGIN) {
            Ok(v) => println!("Latest published version: {v}"),
            Err(e) => println!("Could not query remote version: {e:#}"),
        }
    }
    Ok(())
}

/// Run `<path> --version` and return the first stdout line. Used both
/// here and by self-update's smoke test (which has its own copy to
/// keep the install modules self-contained); the parsing is the same
/// "clap-style version line" shape regardless of which binary it is.
fn query_version(path: &std::path::Path) -> Result<String> {
    let out = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("running `{} --version`", path.display()))?;
    if !out.status.success() {
        bail!(
            "{} --version exited with {}",
            path.display(),
            out.status
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let first = stdout.lines().next().unwrap_or("").trim();
    if first.is_empty() {
        bail!("empty --version output");
    }
    Ok(first.to_owned())
}

/// Fetch `<origin>/latest/install.sh` and pull the version out of the
/// `# samizdat-up bootstrap installer (X.Y.Z)` header that the publish
/// workflow stamps in. The shim is small (a few KB) and is the one
/// file the collection guarantees to keep at a stable path, so it is
/// the cheapest "what is the current version" probe available without
/// a directory-listing endpoint.
fn latest_remote_version(origin: &str) -> Result<String> {
    let url = format!("{origin}/latest/install.sh");
    let body = if let Some(local_path) = strip_file_scheme(&url) {
        std::fs::read_to_string(&local_path)
            .with_context(|| format!("reading local install.sh at {}", local_path.display()))?
    } else {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;
        let resp = client
            .get(&url)
            .send()
            .with_context(|| format!("requesting {url}"))?;
        if !resp.status().is_success() {
            bail!("GET {url} returned HTTP {}", resp.status());
        }
        resp.text().with_context(|| format!("reading body of {url}"))?
    };
    parse_version_from_install_sh(&body)
        .with_context(|| format!("could not parse version from {url}"))
}

fn parse_version_from_install_sh(body: &str) -> Result<String> {
    const PREFIX: &str = "# samizdat-up bootstrap installer (";
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(PREFIX) {
            if let Some(end) = rest.find(')') {
                let v = rest[..end].trim();
                if !v.is_empty() {
                    return Ok(v.to_owned());
                }
            }
        }
    }
    bail!("no `{PREFIX}X.Y.Z)` header in install.sh")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_from_real_header() {
        let body = "#! /usr/bin/env bash\n\
                    #\n\
                    # samizdat-up bootstrap installer (0.1.0)\n\
                    #\n\
                    set -eu\n";
        assert_eq!(parse_version_from_install_sh(body).unwrap(), "0.1.0");
    }

    #[test]
    fn rejects_install_sh_without_version_header() {
        let body = "#! /usr/bin/env bash\nset -eu\n";
        assert!(parse_version_from_install_sh(body).is_err());
    }

    #[test]
    fn rejects_unterminated_header() {
        let body = "# samizdat-up bootstrap installer (0.1.0\n";
        assert!(parse_version_from_install_sh(body).is_err());
    }
}
