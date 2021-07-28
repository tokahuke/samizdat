use std::borrow::Cow;

pub trait Returnable {
    fn content_type(&self) -> Cow<str> {
        "text/plain".into()
    }

    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::OK
    }

    fn render(&self) -> Cow<str>;
}

impl Returnable for () {
    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::NO_CONTENT
    }

    fn render(&self) -> Cow<str> {
        "".into()
    }
}

impl Returnable for &str {
    fn render(&self) -> Cow<str> { (*self).into() }
}

impl Returnable for String {
    fn render(&self) -> Cow<str> { self.into() }
}

impl<'a, T> Returnable for &'a T 
where 
    T: Returnable
{
    fn content_type(&self) -> Cow<str> {
        (*self).content_type()
    }

    fn status_code(&self) -> http::StatusCode {
        (*self).status_code()
    }

    fn render(&self) -> Cow<str> {
        (*self).render()
    }
}

impl<T> Returnable for Option<T> 
where
    T: Returnable
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
    
    fn render(&self) -> Cow<str> {
        match self {
            Some(thing) => thing.render(),
            None => "not found".into(),
        }
    }
}

impl<T, E> Returnable for Result<T, E>
where
    T: Returnable,
    E: Returnable
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
    
    fn render(&self) -> Cow<str> {
        match self {
            Ok(thing) => thing.render(),
            Err(err) => err.render(),
        }
    }
}

impl Returnable for Vec<u8> {
    fn render(&self) -> Cow<str> {
        String::from_utf8_lossy(self)
    }
}
