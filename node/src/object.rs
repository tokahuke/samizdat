use serde_derive::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Object<'a> {
    pub content_type: &'a str,
    pub content: &'a [u8],
}

impl<'a> Object<'a> {
    pub fn new(content_type: &'a str, content: &'a [u8]) -> Object<'a> {
        Object { content_type, content }
    }
}
