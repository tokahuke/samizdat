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
use serde::Deserialize;
use std::collections::BTreeMap;
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
/// Remote listing: fetch the collection's signed `_inventory` (a JSON
/// object mapping every published path to its content hash) and pull
/// the set of versions out of the first path segment. This is the
/// same inventory the proxy resolves objects against, so it is
/// always in sync with what is actually fetchable.
pub fn list_versions(remote: bool) -> Result<()> {
    // version -> binary names at that version. Used in the remote
    // section to annotate a published version with EXACTLY which
    // local binaries are pinned to it -- different binaries can be at
    // different versions (e.g. samizdat-up updated but daemons not
    // yet), and lumping them into a single "installed" tag would lie.
    // version -> binary names at that version. Populated from
    // `installed_binary_paths()`; the running samizdat-up's own
    // version is reported on the header line below and does not get
    // its own entry (the installed samizdat-up at the platform's
    // standard path is enumerated like any other binary).
    let mut bins_at: std::collections::BTreeMap<String, Vec<&'static str>> =
        std::collections::BTreeMap::new();

    println!("samizdat-up {} (this binary)", env!("CARGO_PKG_VERSION"));

    let installed = crate::install::installed_binary_paths();
    if installed.is_empty() {
        println!("(no Samizdat binaries found in standard install paths)");
    } else {
        for (name, path) in &installed {
            let raw = query_version(path).unwrap_or_else(|_| "unknown".to_owned());
            // clap reports "<bin-name> X.Y.Z"; the binary name is
            // already in the first column, so collapse to just the
            // version token for readability.
            let version = raw
                .split_whitespace()
                .nth(1)
                .unwrap_or(&raw)
                .to_owned();
            if version != "unknown" {
                bins_at.entry(version.clone()).or_default().push(name);
            }
            println!("{name:<14} {version} (at {})", path.display());
        }
    }

    if remote {
        println!();
        match fetch_remote_inventory(DEFAULT_ORIGIN) {
            Ok(doc) => {
                let versions = enumerate_versions(&doc);
                let latest = resolve_latest_alias(&doc);
                if versions.is_empty() {
                    println!("No versions in the published inventory.");
                } else {
                    println!("Published versions:");
                    for v in &versions {
                        let mut tags: Vec<String> = Vec::new();
                        if latest.as_deref() == Some(v.as_str()) {
                            tags.push("latest".to_owned());
                        }
                        if let Some(bins) = bins_at.get(v) {
                            tags.push(format!("installed: {}", bins.join(", ")));
                        }
                        if tags.is_empty() {
                            println!("    {v}");
                        } else {
                            println!("    {v}  (= {})", tags.join("; "));
                        }
                    }
                    if latest.is_none() {
                        println!("(`latest` alias did not match any published version)");
                    }
                }
            }
            Err(e) => println!("Could not query remote inventory: {e:#}"),
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

/// Minimal shape of the collection's `_inventory` JSON, just enough
/// to extract path->hash pairs. The full type lives in
/// `node/src/models/collection.rs` (`Inventory`), but pulling that in
/// would drag the whole node DB/model layer into samizdat-up; the
/// on-the-wire JSON is stable enough to parse with a 4-line struct.
#[derive(Debug, Deserialize)]
struct InventoryDoc {
    inventory: BTreeMap<String, String>,
}

/// Fetch `<origin>/_inventory` -- the inventory object that ships with
/// every base edition of the collection (see `Inventory` in
/// `node/src/models/collection.rs`). The proxy serves it like any
/// other content-addressed object; we just decode the JSON.
fn fetch_remote_inventory(origin: &str) -> Result<InventoryDoc> {
    let url = format!("{origin}/_inventory");
    let body = if let Some(local_path) = strip_file_scheme(&url) {
        std::fs::read_to_string(&local_path)
            .with_context(|| format!("reading local inventory at {}", local_path.display()))?
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
    serde_json::from_str(&body).with_context(|| format!("parsing inventory JSON from {url}"))
}

/// First path segment of every entry, minus the `latest` alias. The
/// inventory groups files under `<version>/...` and `latest/...`, so
/// the set of distinct first segments IS the set of published
/// versions.
fn enumerate_versions(doc: &InventoryDoc) -> Vec<String> {
    let mut versions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for path in doc.inventory.keys() {
        let first = path.split('/').next().unwrap_or("");
        if first.is_empty() || first == "latest" {
            continue;
        }
        versions.insert(first.to_owned());
    }
    versions.into_iter().collect()
}

/// Resolve which concrete version `latest/` aliases to. The publish
/// workflow places identical objects at `latest/...` and
/// `<version>/...`, so they share content hashes; pick a sentinel path
/// that exists for every version (`install.sh`, which the bootstrap
/// shim sits at) and find the version whose hash matches `latest`'s.
fn resolve_latest_alias(doc: &InventoryDoc) -> Option<String> {
    let target = doc.inventory.get("latest/install.sh")?;
    for (path, hash) in &doc.inventory {
        if hash != target {
            continue;
        }
        if let Some(version) = path.strip_suffix("/install.sh") {
            if version != "latest" && !version.contains('/') {
                return Some(version.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(pairs: &[(&str, &str)]) -> InventoryDoc {
        InventoryDoc {
            inventory: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn enumerate_strips_latest_and_dedupes() {
        let d = doc(&[
            ("0.1.0/install.sh", "h-installsh"),
            ("0.1.0/x86_64-unknown-linux-gnu/node/samizdat", "h-cli"),
            ("0.2.0/install.sh", "h-installsh-2"),
            ("latest/install.sh", "h-installsh-2"),
            ("latest/x86_64-unknown-linux-gnu/node/samizdat", "h-cli-2"),
        ]);
        assert_eq!(enumerate_versions(&d), vec!["0.1.0", "0.2.0"]);
    }

    #[test]
    fn enumerate_empty_inventory_returns_empty() {
        let d = doc(&[]);
        assert!(enumerate_versions(&d).is_empty());
    }

    #[test]
    fn resolve_latest_via_install_sh_hash() {
        let d = doc(&[
            ("0.1.0/install.sh", "h-a"),
            ("0.2.0/install.sh", "h-b"),
            ("latest/install.sh", "h-b"),
        ]);
        assert_eq!(resolve_latest_alias(&d), Some("0.2.0".to_owned()));
    }

    #[test]
    fn resolve_latest_returns_none_when_no_match() {
        let d = doc(&[
            ("0.1.0/install.sh", "h-a"),
            ("latest/install.sh", "h-detached"),
        ]);
        assert_eq!(resolve_latest_alias(&d), None);
    }

    #[test]
    fn resolve_latest_returns_none_when_alias_missing() {
        let d = doc(&[("0.1.0/install.sh", "h-a")]);
        assert_eq!(resolve_latest_alias(&d), None);
    }
}
