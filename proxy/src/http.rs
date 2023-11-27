use mime::Mime;
use warp::path::FullPath;
use warp::Filter;

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

pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    crate::balanced_or_tree!(proxy())
}

pub fn proxy() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path::full()
        .and(warp::get())
        .and_then(|path: FullPath| async move {
            thread_local! {
                static CLIENT: reqwest::Client = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .unwrap();
            }

            // Get entity and content hash from page path.
            let mut split = path.as_str().split('/');
            split.next().expect("always starts with /, right?");
            let (entity, content_hash) = match (split.next(), split.next()) {
                (None, None) => return Err(warp::reject()),
                (Some(identity), None) => ("_identity", identity),
                (Some(entity), Some(content_hash)) if entity.starts_with('_') => {
                    (entity, content_hash)
                }
                (Some(identity), Some(_)) => ("_identity", identity),
                (None, Some(_)) => unreachable!(),
            };

            // Query node for the web page:
            let translated = format!("http://localhost:4510{}", path.as_str());
            let response = CLIENT
                .with(|client| client.get(translated).send())
                .await
                .unwrap();

            let response = match response.status().as_u16() {
                status @ 300..=399 => http::Response::builder()
                    .status(status)
                    .header("Location", response.headers().get("Location").unwrap())
                    .body(hyper::body::Body::empty()),
                status => {
                    let content_type = response
                        .headers()
                        .get("Content-Type")
                        .cloned()
                        .unwrap_or_else(|| "text/plain".parse().expect("is valid header"));
                    let mut response_builder = http::Response::builder()
                        .status(status)
                        .header("Content-Type", content_type.clone());

                    for &header in PROXY_HEADERS {
                        if let Some(value) = response.headers().get(header) {
                            response_builder = response_builder.header(header, value);
                        }
                    }

                    // If web page, do your shenanigans:
                    let mime: Mime = content_type.to_str().unwrap_or_default().parse().unwrap();
                    let proxied = if mime == mime::TEXT_HTML_UTF_8 || mime == mime::TEXT_HTML {
                        let body = response.bytes().await.unwrap();
                        hyper::body::Body::from(proxy_page(body.as_ref(), entity, content_hash))
                    } else {
                        hyper::body::Body::wrap_stream(response.bytes_stream())
                    };

                    response_builder.body(proxied)
                }
            };

            // HACK! Type system won't comply.
            if true {
                Ok(response)
            } else {
                Err(warp::reject())
            }
        })
}
