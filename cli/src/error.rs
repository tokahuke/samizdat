use failure_derive::Fail;
use std::io;

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "{}", _0)]
    Message(String),
    #[fail(display = "http client error: {}", _0)]
    Reqwest(reqwest::Error),
    #[fail(display = "io error: {}", _0)]
    Io(io::Error),
    #[fail(display = "deserialize error: {}", _0)]
    DeserializeJson(serde_json::Error),
    #[fail(display = "deserialize error: {}", _0)]
    DeserializeToml(toml::de::Error),
    #[fail(display = "notify error: {}", _0)]
    NotifyError(notify::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Error {
        Error::Reqwest(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::DeserializeJson(e)
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Error {
        Error::DeserializeToml(e)
    }
}

impl From<notify::Error> for Error {
    fn from(e: notify::Error) -> Error {
        Error::NotifyError(e)
    }
}

impl From<crate::Error> for String {
    fn from(e: crate::Error) -> String {
        e.to_string()
    }
}

impl From<&'static str> for Error {
    fn from(e: &'static str) -> Error {
        Error::Message(e.to_string())
    }
}
