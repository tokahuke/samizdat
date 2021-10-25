use lazy_static::lazy_static;
use regex::Regex;
use std::borrow::Cow;
use std::path::Path;

lazy_static! {
    static ref MATCH_HTML: Regex = Regex::new(r#"\.html?$"#).expect("valid regex");
    static ref FIND_HREF: Regex = Regex::new(r#"href\s*=\s*('|")/"#).expect("valid regex");
}

fn proxy_page(raw: Cow<str>) -> String {
    FIND_HREF.replace_all(&*raw, "href=$1~/").into_owned()
}

pub fn maybe_proxy_page(path: impl AsRef<Path>, raw: &'_ [u8]) -> Cow<'_, [u8]> {
    if MATCH_HTML.is_match(&*path.as_ref().to_string_lossy()) {
        proxy_page(String::from_utf8_lossy(raw)).into_bytes().into()
    } else {
        Cow::Borrowed(raw)
    }
}
