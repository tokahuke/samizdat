use mime::Mime;
use warp::path::Tail;
use warp::Filter;

use crate::html::proxy_page;

pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    crate::balanced_or_tree!(proxy())
}

pub fn proxy() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!(String / String / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(
            |entity: String, content_hash: String, tail: Tail| async move {
                thread_local! {
                    static CLIENT: reqwest::Client = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .unwrap();
                }

                // Query node for the web page:
                let translated = format!(
                    "http://localhost:4510/{}/{}/{}",
                    entity,
                    content_hash,
                    tail.as_str()
                );
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
                        let body = response.bytes().await.unwrap();

                        // If web page, do your shenanigans:
                        let mime: Mime = content_type.to_str().unwrap_or_default().parse().unwrap();
                        let proxied = if mime == mime::TEXT_HTML_UTF_8 || mime == mime::TEXT_HTML {
                            proxy_page(body.as_ref(), &entity, &content_hash)
                        } else {
                            body
                        };

                        // Builsd response:
                        http::Response::builder()
                            .status(status)
                            .header("Content-Type", content_type)
                            .body(hyper::body::Body::from(proxied))
                    }
                };

                // HACK! Type system won't comply.
                if true {
                    Ok(response)
                } else {
                    Err(warp::reject())
                }
            },
        )
}
