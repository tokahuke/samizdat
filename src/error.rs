use base64_url::base64;
use failure_derive::Fail;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "message: {}", _0)]
    Message(String),
    #[fail(display = "base64 decode error: {}", _0)]
    Base64(base64::DecodeError),
}

impl From<base64::DecodeError> for Error {
    fn from(e: base64::DecodeError) -> Error {
        Error::Base64(e)
    }
}

impl From<String> for Error {
    fn from(e: String) -> Error {
        Error::Message(e)
    }
}

impl Error {
    pub fn status_code(&self) -> http::StatusCode {
        http::StatusCode::BAD_REQUEST
    }
}
