use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::OriginalUri;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use mime::Mime;

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
    let path = uri.path();
    let mut split = path.split('/');
    split.next().expect("always starts with /, right?");
    let (entity, content_hash) = match (split.next(), split.next()) {
        (None, None) => todo!(),
        (Some(identity), None) => ("_identity", identity),
        (Some(entity), Some(content_hash)) if entity.starts_with('_') => (entity, content_hash),
        (Some(identity), Some(_)) => ("_identity", identity),
        (None, Some(_)) => unreachable!(),
    };

    // Query node for the web page:
    let translated = format!("{}{}", cli().node, path);
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

            // If web page, do your shenanigans:
            let mime: Mime = content_type.to_str().unwrap_or_default().parse()?;

            if mime == mime::TEXT_HTML_UTF_8 || mime == mime::TEXT_HTML {
                let body = response.bytes().await?;
                response_builder.body(proxy_page(body.as_ref(), entity, content_hash).into())?
            } else {
                response_builder.body(response.bytes().await?.into())?
            }
        }
    };

    Ok(response)
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
