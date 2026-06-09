//! HTML content processing for `samizdat commit` and `samizdat watch`.
//!
//! Each series and identity has its own browser origin via subdomain
//! (`series-<key>.<root>` / `<identity>.<root>`). Absolute paths
//! resolve against the entity's own root, so commit-time path
//! rewriting is no longer needed; this module only injects the
//! page-refresh snippet that `samizdat watch` uses in dev mode.

use regex::Regex;
use std::{borrow::Cow, net::SocketAddr, path::Path};

use std::sync::LazyLock;

/// Matches `.html` and `.htm` file extensions.
static MATCH_HTML: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.html?$"#).expect("valid regex"));

/// Matches the closing `</body>` tag.
static FIND_FOOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"</body>"#).expect("valid regex"));

/// If `path` is an HTML file and `refresh_server_addr` is set, injects the
/// `samizdat watch` reload snippet before `</body>`. Otherwise returns
/// the bytes unchanged.
pub fn proxy_page(
    path: impl AsRef<Path>,
    raw: &'_ [u8],
    refresh_server_addr: Option<SocketAddr>,
) -> Cow<'_, [u8]> {
    if MATCH_HTML.is_match(&path.as_ref().to_string_lossy()) {
        let Some(addr) = refresh_server_addr else {
            return Cow::Borrowed(raw);
        };
        let raw = String::from_utf8_lossy(raw);
        FIND_FOOT
            .replace_all(
                raw.as_ref(),
                concat!(include_str!("trigger_refresh_snippet.html"), "</body>")
                    .replace("$$address", &addr.to_string()),
            )
            .into_owned()
            .into_bytes()
            .into()
    } else {
        // Not HTML: make no changes.
        Cow::Borrowed(raw)
    }
}
