use lazy_static::lazy_static;
use regex::Regex;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::path::Path;

lazy_static! {
    static ref MATCH_HTML: Regex = Regex::new(r#"\.html?$"#).expect("valid regex");
    static ref FIND_HREF: Regex = Regex::new(r#"href\s*=\s*('|")/"#).expect("valid regex");
    static ref FIND_FOOT: Regex = Regex::new(r#"</body>"#).expect("valid regex");
}

pub fn proxy_page(
    path: impl AsRef<Path>,
    raw: &'_ [u8],
    refresh_server_addr: Option<SocketAddr>,
) -> Cow<'_, [u8]> {
    if MATCH_HTML.is_match(&*path.as_ref().to_string_lossy()) {
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
