use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use mime::Mime;
use samizdat_common::host_label::is_base32_key_label;
use samizdat_common::identity::check_servable_identity;

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

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("failed to build HTTP client")
});

/// Build the host-form router. Dispatches purely on the `Host` header
/// and forwards the request path verbatim.
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
    let host = match axum::http::uri::Authority::try_from(raw_host.trim()) {
        Ok(authority) => authority.host().to_ascii_lowercase(),
        Err(_) => return Ok(bad_request("malformed Host header")),
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
