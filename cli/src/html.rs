//! HTML content processing and transformation for serving pages in development mode.
//!
//! This module handles HTML file modifications for the CLI, including path proxying and
//! automatic page refresh functionality. It processes HTML files to adjust internal links
//! and inject refresh-triggering JavaScript when needed.

use regex::Regex;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::Path;

use std::sync::LazyLock;

/// Regular expression to match HTML file extensions.
static MATCH_HTML: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.html?$"#).expect("valid regex"));

/// Regular expression to match href attributes that are relative paths.
static FIND_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"href\s*=\s*('|")/"#).expect("valid regex"));

/// Regular expression to match the closing body tag.
static FIND_FOOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"</body>"#).expect("valid regex"));

/// Processes an HTML page, adjusting paths and optionally adding refresh functionality.
///
/// # Arguments
/// * `path` - The file path to process
/// * `raw` - The raw content of the file
/// * `refresh_server_addr` - Optional WebSocket server address for refresh functionality
///
/// # Returns
/// The processed content, either modified or unchanged if not an HTML file
pub fn proxy_page(
    path: impl AsRef<Path>,
    raw: &'_ [u8],
    refresh_server_addr: Option<SocketAddr>,
) -> Cow<'_, [u8]> {
    if MATCH_HTML.is_match(&path.as_ref().to_string_lossy()) {
        // Only support utf-8 HTML by now...
        let raw = String::from_utf8_lossy(raw);

        // Proxy paths in HTML
        let proxied = FIND_HREF.replace_all(raw.as_ref(), "href=$1~/");

        // Add the page refresh snippet to the end of the body of the HTML.
        if let Some(addr) = refresh_server_addr {
            FIND_FOOT.replace_all(
                proxied.as_ref(),
                concat!(include_str!("trigger_refresh_snippet.html"), "</body>")
                    .replace("$$address", &addr.to_string()),
            )
        } else {
            proxied
        }
        .into_owned()
        .into_bytes()
        .into()
    } else {
        // Not HTML: make no changes.
        Cow::Borrowed(raw)
    }
}
