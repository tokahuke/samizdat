use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use mime::Mime;
use samizdat_common::host_label::{encode_key_to_host_label, is_base32_key_label};
use samizdat_common::identity::check_servable_identity;
use samizdat_common::Key;

use crate::cli::cli;
use crate::html::proxy_page;

const PROXY_HEADERS: &[&str] = &[
    "ETag",
    "X-Samizdat-Bookmark",
    "X-Samizdat-Object",
    "X-Samizdat-Is-Draft",
    "X-Samizdat-Collection",
    "X-Samizdat-Series",
    "X-Samizdat-Edition",
    "X-Samizdat-Query-Duration",
    // Forward the node's security headers to external viewers so the
    // proxied page gets the same protections as a local visit.
    "X-Content-Type-Options",
    "X-Frame-Options",
    "Referrer-Policy",
    "Permissions-Policy",
];

pub fn api() -> axum::Router {
    Router::new()
        .route("/{*path}", get(proxy))
        .route("/", get(proxy))
        .layer(tower::ServiceBuilder::new().layer(tower_http::trace::TraceLayer::new_for_http()))
}

pub async fn proxy(original_uri: OriginalUri) -> Response<Body> {
    match do_proxy(original_uri).await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!("Server error: {err:?}");
            Response::builder()
                .status(500)
                .body(bytes::Bytes::from_static(b"500 Internal Server Error").into())
                .expect("can build internal server error message")
        }
    }
}

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

pub async fn do_proxy(OriginalUri(uri): OriginalUri) -> Result<Response<Body>, anyhow::Error> {
    // Get entity and content hash from page path.
    //
    // `uri.path()` from axum always starts with `/`, so `split('/').next()` is
    // always `Some("")`; the previous version had an `.expect()` and a
    // `todo!()` arm for impossible cases. Re-shape as concrete matches that
    // return a clean 400 if a future routing change ever feeds us an
    // unexpected path, rather than panicking the request thread.
    let path = uri.path();
    let mut split = path.split('/');
    split.next(); // leading empty segment from the `/`
    let (entity, content_hash) = match (split.next(), split.next()) {
        // Root request, no entity. Treat as the "samizdat home" path.
        (None, _) | (Some(""), None) => ("_identity", ""),
        (Some(entity), Some(content_hash)) if entity.starts_with('_') => (entity, content_hash),
        // Otherwise the first segment is an identity name (no leading `_`).
        (Some(identity), _) => ("_identity", identity),
    };

    // The node serves content at `<base32-key>.localhost:<port>/<rest>`
    // and `<identity>.localhost:<port>/<rest>`. The proxy keeps the
    // path-form on its external surface and rewrites here into the
    // host-form upstream.
    let translated = match translate_to_node_url(path, &cli().node) {
        Ok(url) => url,
        Err(BadIdentity { handle, reason }) => {
            tracing::info!("Rejecting identity '{handle}' at proxy: {reason}");
            return Ok(axum::response::Response::builder()
                .status(400)
                .header("Content-Type", "text/plain")
                .body(
                    format!(
                        "identity '{handle}' is not servable as a subdomain: {reason}. \
                         Use the public-key form `_series/<base64-key>/` if you know it."
                    )
                    .into(),
                )
                .expect("can build 400 response"));
        }
    };
    let response = CLIENT.get(translated).send().await?;

    let response = match response.status().as_u16() {
        status @ 300..=399 => axum::response::Response::builder()
            .status(status)
            .header(
                "Location",
                response
                    .headers()
                    .get("Location")
                    .ok_or_else(|| anyhow::anyhow!("Missing location header in redirect"))?,
            )
            .body(hyper::body::Bytes::default().into())?,
        status => {
            let content_type = response
                .headers()
                .get("Content-Type")
                .cloned()
                .unwrap_or_else(|| "text/plain".parse().expect("is valid header"));
            let mut response_builder = axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type.clone());

            for &header in PROXY_HEADERS {
                if let Some(value) = response.headers().get(header) {
                    response_builder = response_builder.header(header, value);
                }
            }

            // If web page, do your shenanigans. Compare on `type_`/`subtype`
            // rather than `==`-against `mime::TEXT_HTML_UTF_8` so that
            // `text/html; charset=utf-8`, `text/html; charset=US-ASCII`,
            // `text/html` (no charset) all take the HTML path.
            let mime: Mime = content_type.to_str().unwrap_or_default().parse()?;

            if mime.type_() == mime::TEXT && mime.subtype() == mime::HTML {
                let body = response.bytes().await?;
                response_builder.body(proxy_page(body.as_ref(), entity, content_hash).into())?
            } else {
                response_builder.body(response.bytes().await?.into())?
            }
        }
    };

    Ok(response)
}

/// Reason a `/~<identity>/...` request cannot be forwarded to the node.
struct BadIdentity {
    handle: String,
    reason: samizdat_common::identity::Reason,
}

/// Rewrites an incoming proxy path into the upstream node URL. Three cases:
///
/// - `/_series/<base64-key>/<rest>` -> `<scheme>://<base32-key>.<node-host>:<node-port>/<rest>`.
///   The key is decoded as URL-safe base64 (matching `samizdat_common::Key::FromStr`)
///   and re-encoded as lowercase base32 for the subdomain label. Bad keys
///   are passed through verbatim (the node will 400 or 404, with detail),
///   so this function never fails on key parse alone.
/// - `/~<identity>/<rest>` -> `<scheme>://<identity>.<node-host>:<node-port>/<rest>`.
///   The identity is validated with `check_servable_identity` first; failures
///   return Err so the caller can produce a clean 400 to the external user.
/// - Anything else (root, ACME paths) -> forwarded verbatim.
fn translate_to_node_url(path: &str, node_base: &str) -> Result<String, BadIdentity> {
    // Parse the node base URL once so we can substitute the host portion.
    // If the operator passed an invalid `--node` value, fall back to the
    // verbatim concatenation; we cannot do anything better here and the
    // request will surface a connect error downstream.
    let Ok(base_url) = url::Url::parse(node_base) else {
        return Ok(format!("{node_base}{path}"));
    };
    let scheme = base_url.scheme();
    let host = match base_url.host_str() {
        Some(h) => h,
        None => return Ok(format!("{node_base}{path}")),
    };
    let port_part = match base_url.port() {
        Some(p) => format!(":{p}"),
        None => String::new(),
    };

    // `/_series/<key>/<rest>`
    if let Some(after) = path.strip_prefix("/_series/") {
        let mut split = after.splitn(2, '/');
        let key_str = split.next().unwrap_or("");
        let rest = split.next().unwrap_or("");
        if let Ok(key) = key_str.parse::<Key>() {
            let label = encode_key_to_host_label(&key);
            return Ok(format!("{scheme}://{label}.{host}{port_part}/{rest}"));
        }
        // Bad key: fall through to verbatim forward so the node can answer.
    }

    // `/~<identity>/<rest>` or `/~<identity>`
    if let Some(after) = path.strip_prefix("/~") {
        let mut split = after.splitn(2, '/');
        let handle = split.next().unwrap_or("").to_ascii_lowercase();
        let rest = split.next().unwrap_or("");
        if handle.is_empty() {
            return Ok(format!("{node_base}{path}"));
        }
        if let Err(reason) = check_servable_identity(&handle) {
            return Err(BadIdentity { handle, reason });
        }
        return Ok(format!("{scheme}://{handle}.{host}{port_part}/{rest}"));
    }

    // Root / ACME / anything else: verbatim.
    Ok(format!("{node_base}{path}"))
}

/// Build the host-form router used in wildcard-cert mode. Unlike `api`,
/// which rewrites `/_series/...` / `/~identity/...` path-form requests
/// into the upstream node's host-form, this router dispatches purely on
/// the `Host` header and forwards the request path verbatim.
///
/// Three classes of host are accepted:
///
/// - bare `<wildcard_root>`: forwards path verbatim to the node root.
///   This serves the proxy's landing / welcome surface.
/// - `<base32-key>.<wildcard_root>`: upstream is
///   `<scheme>://<base32-key>.<node-host>:<node-port>/<path>`.
/// - `<identity>.<wildcard_root>` where `<identity>` passes
///   `check_servable_identity`: upstream is
///   `<scheme>://<identity>.<node-host>:<node-port>/<path>`.
///
/// Anything else gets a 400.
pub fn wildcard_api(wildcard_root: String) -> axum::Router {
    let state = WildcardState { wildcard_root };
    Router::new()
        .route("/{*path}", get(wildcard_dispatch))
        .route("/", get(wildcard_dispatch))
        .layer(tower::ServiceBuilder::new().layer(tower_http::trace::TraceLayer::new_for_http()))
        .with_state(std::sync::Arc::new(state))
}

#[derive(Clone)]
struct WildcardState {
    wildcard_root: String,
}

async fn wildcard_dispatch(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<WildcardState>>,
    headers: HeaderMap,
    original_uri: OriginalUri,
) -> Response<Body> {
    match do_wildcard_dispatch(&state.wildcard_root, &headers, original_uri).await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!("Server error: {err:?}");
            Response::builder()
                .status(500)
                .body(bytes::Bytes::from_static(b"500 Internal Server Error").into())
                .expect("can build internal server error message")
        }
    }
}

/// Outcome of inspecting the `Host` header in wildcard mode.
enum WildcardHost<'a> {
    /// Host equals the configured wildcard root: forward verbatim to
    /// the node's bare host.
    Bare,
    /// Host is `<sub>.<wildcard_root>`: forward to the matching
    /// subdomain on the node.
    Sub(&'a str),
}

async fn do_wildcard_dispatch(
    wildcard_root: &str,
    headers: &HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response<Body>, anyhow::Error> {
    let raw_host = match headers.get("host").and_then(|h| h.to_str().ok()) {
        Some(h) => h,
        None => return Ok(bad_request("missing or malformed Host header")),
    };
    let host = match strip_host_port(raw_host) {
        Some(h) => h.to_ascii_lowercase(),
        None => return Ok(bad_request("malformed Host header")),
    };
    let wildcard_root_lc = wildcard_root.to_ascii_lowercase();

    let classified = if host == wildcard_root_lc {
        WildcardHost::Bare
    } else if let Some(prefix) = host.strip_suffix(&format!(".{wildcard_root_lc}")) {
        if prefix.is_empty() || prefix.contains('.') {
            return Ok(bad_request("untrusted host"));
        }
        WildcardHost::Sub(prefix)
    } else {
        return Ok(bad_request("untrusted host"));
    };

    // Path + query (axum's `OriginalUri.path()` does NOT include the
    // query string; reconstruct from `path_and_query`).
    let path_and_query = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| uri.path());

    let (entity, content_hash) = entity_from_path(uri.path());

    // The proxy's `--node` URL is the node's bare admin origin
    // (e.g. `http://localhost:4510`). For host-form forwarding we
    // substitute the host label.
    let node = node_base()
        .ok_or_else(|| anyhow::anyhow!("invalid --node URL: {}", cli().node))?;

    let upstream = match classified {
        WildcardHost::Bare => format!("{}{}", cli().node, path_and_query),
        WildcardHost::Sub(sub) => {
            let label = if is_base32_key_label(sub) {
                sub.to_owned()
            } else if check_servable_identity(sub).is_ok() {
                sub.to_owned()
            } else {
                return Ok(bad_request("untrusted host"));
            };
            format!(
                "{}://{}.{}{}{}",
                node.scheme, label, node.host, node.port_part, path_and_query
            )
        }
    };

    let response = CLIENT.get(upstream).send().await?;

    let response = match response.status().as_u16() {
        status @ 300..=399 => axum::response::Response::builder()
            .status(status)
            .header(
                "Location",
                response
                    .headers()
                    .get("Location")
                    .ok_or_else(|| anyhow::anyhow!("Missing location header in redirect"))?,
            )
            .body(hyper::body::Bytes::default().into())?,
        status => {
            let content_type = response
                .headers()
                .get("Content-Type")
                .cloned()
                .unwrap_or_else(|| "text/plain".parse().expect("is valid header"));
            let mut response_builder = axum::response::Response::builder()
                .status(status)
                .header("Content-Type", content_type.clone());

            for &header in PROXY_HEADERS {
                if let Some(value) = response.headers().get(header) {
                    response_builder = response_builder.header(header, value);
                }
            }

            let mime: Mime = content_type.to_str().unwrap_or_default().parse()?;

            if mime.type_() == mime::TEXT && mime.subtype() == mime::HTML {
                let body = response.bytes().await?;
                response_builder.body(proxy_page(body.as_ref(), entity, content_hash).into())?
            } else {
                response_builder.body(response.bytes().await?.into())?
            }
        }
    };

    Ok(response)
}

/// Best-effort path -> (entity, content_hash) extraction for the
/// HTML rewriter's CSS namespace. Mirrors the logic in `do_proxy` but
/// trimmed to what `proxy_page` actually consumes (both args are
/// currently unused inside the rewriter; this keeps parity with the
/// path-form code in case that changes).
fn entity_from_path(path: &str) -> (&'static str, &'static str) {
    let _ = path;
    ("_identity", "")
}

/// Decomposed `--node` URL parts used to build upstream URLs. Parsed
/// once at first use and cached for the life of the process.
struct NodeBase {
    scheme: &'static str,
    host: &'static str,
    port_part: &'static str,
}

fn node_base() -> Option<NodeBase> {
    static PARSED: LazyLock<Option<(String, String, String)>> = LazyLock::new(|| {
        let url = url::Url::parse(&cli().node).ok()?;
        let host = url.host_str()?.to_owned();
        let scheme = url.scheme().to_owned();
        let port_part = url.port().map(|p| format!(":{p}")).unwrap_or_default();
        Some((scheme, host, port_part))
    });
    let (scheme, host, port_part) = PARSED.as_ref()?;
    // Safe to leak to &'static because PARSED is a `LazyLock` itself
    // already 'static; `as_str` on the inner String gives us a
    // reference with the same lifetime.
    Some(NodeBase {
        scheme: scheme.as_str(),
        host: host.as_str(),
        port_part: port_part.as_str(),
    })
}

fn bad_request(msg: &'static str) -> Response<Body> {
    Response::builder()
        .status(400)
        .header("Content-Type", "text/plain")
        .body(bytes::Bytes::from_static(msg.as_bytes()).into())
        .expect("can build 400 response")
}

/// Strip the optional `:port` suffix from a Host header value, handling
/// IPv6 brackets. Returns `None` if the value is structurally
/// malformed. Mirrors `node/src/http/host_scope.rs::strip_port`.
fn strip_host_port(raw: &str) -> Option<&str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    if let Some(rest) = raw.strip_prefix('[') {
        let close = rest.find(']')?;
        let addr = &rest[..close];
        let after = &rest[close + 1..];
        if after.is_empty() || after.starts_with(':') {
            return Some(addr);
        }
        return None;
    }

    let mut parts = raw.splitn(2, ':');
    let host = parts.next()?;
    if let Some(port) = parts.next() {
        if port.contains(':') {
            return None;
        }
    }
    Some(host)
}

/// Tests if the node is live at the URL supplied to the CLI.
pub async fn validate_node_is_up() -> Result<(), anyhow::Error> {
    let response = CLIENT.get(format!("{}/", cli().node)).send().await;

    if let Err(error) = response {
        if error.is_connect() {
            anyhow::bail!(
                "Failed to connect to node at {}. Check if samizdat-node is up and running",
                cli().node
            );
        } else {
            anyhow::bail!(
                "Unexpected error testing connection to node at {}: {error}",
                cli().node
            );
        }
    }

    Ok(())
}
