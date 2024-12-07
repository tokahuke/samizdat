use base64_url::base64;
use thiserror::Error;
use std::io;
use tarpc::client::RpcError;

/// Possible errors that can occur within Samizdat.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// One-time general error messages, not intended to be caught and treated.
    #[error("message: {}", _0)]
    Message(String),
    /// Error from the Samizdat RPC.
    #[error("RPC error: {}", _0)]
    Rpc(RpcError),
    /// Error decoding base64 encoded data.
    #[error("base64 decode error: {}", _0)]
    Base64(base64::DecodeError),
    /// Errors from the database.
    #[error("db error: {}", _0)]
    Db(rocksdb::Error),
    /// IO error.
    #[error("io error: {}", _0)]
    Io(io::Error),
    /// Hash representation has the wrong number of bytes.
    #[error("bad hash length (should be 28): {}", _0)]
    BadHashLength(usize),
    /// Error decoding bincode encoded data.
    #[error("decode error: {}", _0)]
    Bincode(Box<bincode::ErrorKind>),
    /// Error connecting with Quic.
    #[error("QUIC connection error: {}", _0)]
    QuicConnectionError(quinn::ConnectionError),
    /// All candidates supplied by the host were unable to fulfill the query.
    #[error("All candidates failed")]
    AllCandidatesFailed,
    /// Invalid collection item
    #[error("invalid collection item")]
    InvalidCollectionItem,
    /// Invalid edition
    #[error("invalid edition")]
    InvalidEdition,
    /// Different public keys.
    #[error("different public keys")]
    DifferentPublicKeys,
    /// No header read.
    #[error("no header read")]
    NoHeaderRead,
    /// A timeout has occurred.
    #[error("timeout")]
    Timeout,
}

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
