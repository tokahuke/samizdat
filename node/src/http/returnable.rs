use std::borrow::Cow;

pub trait Returnable {
    fn content_type(&self) -> Cow<str> {
        "text/plain".into()
    }

    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::OK
    }

    fn render(&self) -> Cow<[u8]>;
}

impl Returnable for () {
    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::NO_CONTENT
    }

    fn render(&self) -> Cow<[u8]> {
        (b"").as_ref().into()
    }
}

impl Returnable for &str {
    fn render(&self) -> Cow<[u8]> {
        (*self).as_bytes().into()
    }
}

impl Returnable for String {
    fn render(&self) -> Cow<[u8]> {
        self.as_bytes().into()
    }
}

impl<T> Returnable for Option<T>
where
    T: Returnable,
{
    fn content_type(&self) -> Cow<str> {
        match self {
            Some(thing) => thing.content_type(),
            None => "text/plain".into(),
        }
    }

    fn status_code(&self) -> http::StatusCode {
        match self {
            Some(thing) => thing.status_code(),
            None => http::StatusCode::NOT_FOUND,
        }
    }

    fn render(&self) -> Cow<[u8]> {
        match self {
            Some(thing) => thing.render(),
            None => b"not found".as_ref().into(),
        }
    }
}

impl<T, E> Returnable for Result<T, E>
where
    T: Returnable,
    E: Returnable,
{
    fn content_type(&self) -> Cow<str> {
        match self {
            Ok(thing) => thing.content_type(),
            Err(err) => err.content_type(),
        }
    }

    fn status_code(&self) -> http::StatusCode {
        match self {
            Ok(thing) => thing.status_code(),
            Err(err) => err.status_code(),
        }
    }

    fn render(&self) -> Cow<[u8]> {
        match self {
            Ok(thing) => thing.render(),
            Err(err) => err.render(),
        }
    }
}

impl Returnable for Vec<u8> {
    fn content_type(&self) -> Cow<str> {
        "octet/stream".into()
    }

    fn render(&self) -> Cow<[u8]> {
        self.into()
    }
}

impl<'a> Returnable for crate::flatbuffers::object::Object<'a> {
    fn content_type(&self) -> Cow<str> {
        self.content_type().into()
    }

    fn render(&self) -> Cow<[u8]> {
        self.content().into()
    }
}

pub struct Return {
    pub content_type: String,
    pub status_code: http::StatusCode,
    pub content: Vec<u8>,
}

impl Returnable for Return {
    fn content_type(&self) -> Cow<str> {
        Cow::Borrowed(&self.content_type)
    }

    fn status_code(&self) -> http::StatusCode {
        self.status_code
    }

    fn render(&self) -> Cow<[u8]> {
        Cow::Borrowed(&self.content)
    }
}
