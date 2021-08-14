use base64_url::base64;
use failure_derive::Fail;
use std::io;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "message: {}", _0)]
    Message(String),
    #[fail(display = "base64 decode error: {}", _0)]
    Base64(base64::DecodeError),
    #[fail(display = "db error: {}", _0)]
    Db(rocksdb::Error),
    #[fail(display = "io error: {}", _0)]
    Io(io::Error),
    #[fail(display = "bad hash length (should be 28): {}", _0)]
    BadHashLength(usize),
    #[fail(display = "decode error: {}", _0)]
    Bincode(Box<bincode::ErrorKind>),
    #[fail(display = "QUIC connection error: {}", _0)]
    QuicConnectionError(quinn::ConnectionError),
    #[fail(display = "All candidates failed")]
    AllCandidatesFailed,
    #[failt(display = "invalid collection item")]
    InvalidCollectionItem,
}

impl warp::reject::Reject for crate::Error {}

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

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<Box<bincode::ErrorKind>> for Error {
    fn from(e: bincode::Error) -> Error {
        Error::Bincode(e)
    }
}

impl From<quinn::ConnectionError> for Error {
    fn from(e: quinn::ConnectionError) -> Error {
        Error::QuicConnectionError(e)
    }
}
