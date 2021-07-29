use base64_url::base64;
use failure_derive::Fail;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "message: {}", _0)]
    Message(String),
    #[fail(display = "base64 decode error: {}", _0)]
    Base64(base64::DecodeError),
    #[fail(display = "db error: {}", _0)]
    Db(rocksdb::Error),
    #[fail(display = "invalid flatbuffer: {}", _0)]
    FlatBuffer(::flatbuffers::InvalidFlatbuffer),
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

impl From<rocksdb::Error> for Error {
    fn from(e: rocksdb::Error) -> Error {
        Error::Db(e)
    }
}

impl From<::flatbuffers::InvalidFlatbuffer> for Error {
    fn from(e: ::flatbuffers::InvalidFlatbuffer) -> Error {
        Error::FlatBuffer(e)
    }
}
