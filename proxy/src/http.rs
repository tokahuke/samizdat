use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use mime::Mime;
use samizdat_common::host_label::encode_key_to_host_label;
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
