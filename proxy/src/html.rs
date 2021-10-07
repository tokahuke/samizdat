use askama::Template;
use lazy_static::lazy_static;
use scraper::{Html, Selector};

lazy_static! {
    static ref SELECT_HEAD: Selector = Selector::parse("head").expect("valid selector");
    static ref SELECT_BODY: Selector = Selector::parse("body").expect("valid selector");
}

const SAMIZDAT_BLOG_PATH: &str = "/_series/fGfgc7ibvwy26U7nHjcaAhYmyLvXl84Ld-qab_0PPJc/install/";

#[derive(Template)]
#[template(path = "proxied-page.html.jinja")]
struct ProxyedPage {
    head: String,
    body: String,
    rand: String,
    download_link: &'static str,
}

pub fn proxy_page(raw: &[u8], _entity: &str, _content_hash: &str) -> bytes::Bytes {
    let source = &String::from_utf8_lossy(raw);
    let html = Html::parse_document(source);
    let head = html
        .select(&SELECT_HEAD)
        .next()
        .map(|head| head.inner_html())
        .unwrap_or_default();
    let body = html
        .select(&SELECT_BODY)
        .next()
        .map(|body| body.inner_html())
        .unwrap_or_default();
    let rand = format!("samizdat_{:x}", rand::random::<u16>());

    ProxyedPage {
        head,
        body,
        rand,
        download_link: SAMIZDAT_BLOG_PATH,
    }
    .render()
    .expect("can always render proxied page")
    .into()
}
