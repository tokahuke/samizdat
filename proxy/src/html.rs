use askama::Template;
use lazy_static::lazy_static;
use scraper::{Html, Selector};

lazy_static! {
    static ref SELECT_TITLE: Selector = Selector::parse("title").expect("valid selector");
    static ref SELECT_META_DESCRIPTION: Selector =
        Selector::parse("meta[name='description']").expect("valid selector");
    static ref SELECT_FAVICON: Selector =
        Selector::parse("link[rel~='icon']").expect("valid selector");
}

#[derive(Template)]
#[template(path = "proxied-page.html.jinja")]
struct ProxyedPage<'a> {
    title: Option<&'a str>,
    meta_description: &'a str,
    favicon: &'a str,
    source: &'a str,
}

pub fn proxy_page(raw: &[u8], entity: &str, content_hash: &str) -> bytes::Bytes {
    let source = &String::from_utf8_lossy(raw);
    let html = Html::parse_document(source);
    let title = html
        .select(&SELECT_TITLE)
        .next()
        .map(|title| title.text().collect::<String>());
    let meta_description = html
        .select(&SELECT_META_DESCRIPTION)
        .next()
        .and_then(|meta_description| meta_description.value().attr("content"))
        .unwrap_or_default();
    let favicon = html
        .select(&SELECT_FAVICON)
        .next()
        .and_then(|favicon_link| favicon_link.value().attr("href"))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("/{}/{}/favicon.ico", entity, content_hash));

    ProxyedPage {
        title: title.as_deref(),
        favicon: &favicon,
        meta_description: &meta_description,
        source,
    }
    .render()
    .expect("can always render proxied page")
    .into()
}
