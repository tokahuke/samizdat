use base64_url::base64;
use failure_derive::Fail;
use std::io;
use tarpc::client::RpcError;

/// Possible errors that can occur within Samizdat.
#[derive(Debug, Fail)]
#[non_exhaustive]
pub enum Error {
    /// One-time general error messages, not intended to be caught and treated.
    #[fail(display = "message: {}", _0)]
    Message(String),
    /// Error from the Samizdat RPC.
    #[fail(display = "RPC error: {}", _0)]
    Rpc(RpcError),
    /// Error decoding base64 encoded data.
    #[fail(display = "base64 decode error: {}", _0)]
    Base64(base64::DecodeError),
    /// Errors from the database.
    #[fail(display = "db error: {}", _0)]
    Db(rocksdb::Error),
    /// IO error.
    #[fail(display = "io error: {}", _0)]
    Io(io::Error),
    /// Hash representation has the wrong number of bytes.
    #[fail(display = "bad hash length (should be 28): {}", _0)]
    BadHashLength(usize),
    /// Error decoding bincode encoded data.
    #[fail(display = "decode error: {}", _0)]
    Bincode(Box<bincode::ErrorKind>),
    /// Error connecting with Quic.
    #[fail(display = "QUIC connection error: {}", _0)]
    QuicConnectionError(quinn::ConnectionError),
    /// All candidates supplied by the host were unable to fulfill the query.
    #[fail(display = "All candidates failed")]
    AllCandidatesFailed,
    /// Invalid collection item
    #[fail(display = "invalid collection item")]
    InvalidCollectionItem,
    /// Invalid edition
    #[fail(display = "invalid edition")]
    InvalidEdition,
    /// Different public keys.
    #[fail(display = "different public keys")]
    DifferentPublicKeys,
    /// No header read.
    #[fail(display = "no header read")]
    NoHeaderRead,
    /// A timeout has occurred.
    #[fail(display = "timeout")]
    Timeout,
}

impl warp::reject::Reject for crate::Error {}

impl From<RpcError> for Error {
    fn from(e: RpcError) -> Error {
        Error::Rpc(e)
    }
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

impl From<&'static str> for Error {
    fn from(e: &'static str) -> Error {
        Error::Message(e.to_string())
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

impl From<Error> for io::Error {
    fn from(e: Error) -> io::Error {
        io::Error::new(io::ErrorKind::Other, e.to_string())
    }
}

impl From<Error> for anyhow::Error {
    fn from(e: Error) -> anyhow::Error {
        anyhow::anyhow!("{e}")
    }
}
