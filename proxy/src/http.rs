use askama::Template;
use lazy_static::lazy_static;
use mime::Mime;
use scraper::{Html, Selector};
use warp::path::FullPath;
use warp::Filter;

lazy_static! {
    static ref SELECT_TITLE: Selector = Selector::parse("title").expect("valid selector");
    static ref SELECT_META_DESCRIPTION: Selector =
        Selector::parse("meta[name='description']").expect("valid selector");
}

#[derive(Template)]
#[template(path = "proxied-page.html.jinja")]
struct ProxyedPage<'a> {
    title: &'a str,
    meta_description: &'a str,
    source: &'a str,
}

pub fn api() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    crate::balanced_or_tree!(proxy(),)
}

pub fn proxy() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path::full()
        .and(warp::get())
        .and_then(|path: FullPath| async move {
            // Query node for the web page:
            let translated = format!("http://localhost:4510{}", path.as_str());
            let response = reqwest::get(translated).await.unwrap();
            let status = response.status();
            let content_type = response
                .headers()
                .get("Content-Type")
                .cloned()
                .unwrap_or_else(|| "text/plain".parse().expect("is valid header"));
            let body = response.bytes().await.unwrap();

            // If web page, do your shenanigans:
            let mime: Mime = content_type.to_str().unwrap_or_default().parse().unwrap();
            let proxied = if mime == mime::TEXT_HTML_UTF_8 || mime == mime::TEXT_HTML {
                let source = &String::from_utf8_lossy(body.as_ref());
                let html = Html::parse_document(source);
                let title = html
                    .select(&SELECT_TITLE)
                    .next()
                    .map(|title| title.text().collect::<String>())
                    .unwrap_or_else(|| path.as_str().to_owned());
                let meta_description = html
                    .select(&SELECT_META_DESCRIPTION)
                    .next()
                    .and_then(|meta_description| meta_description.value().attr("content"))
                    .unwrap_or_default();

                ProxyedPage {
                    title: &title,
                    meta_description: &meta_description,
                    source,
                }
                .render()
                .expect("can always render proxied page")
                .into()
            } else {
                body
            };

            // Buid response:
            let response = http::Response::builder()
                .status(status)
                .header("Content-Type", content_type)
                .body(hyper::body::Body::from(proxied));

            if true {
                Ok(response)
            } else {
                Err(warp::reject())
            }
        })
}