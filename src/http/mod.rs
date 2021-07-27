use std::convert::TryInto;
use std::fmt::Display;
use std::str::FromStr;
use warp::reject::Reject;
use warp::Filter;

use crate::DB;

impl Reject for crate::Error {}

trait Returnable {
    fn content_type(&self) -> &str {
        "text/plain"
    }

    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::OK
    }

    fn render(&self) -> &str;
}

impl Returnable for &str {}

impl Returnable for String {}

impl<T> Returnable for Option<T> {
    fn content_type(&self) -> &str {
        match self {
            Some(thing) => thing.content_type(),
            None => "text/plain",
        }
    }

    fn status_code(&self) -> http::StatusCode {
        match self {
            Some(thing) => thing.status_code(),
            None => http::StatusCode::NOT_FOUND,
        }
    }
}

fn reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: Returnable,
{
    let status_code = if let Err(err) = &t {
        err.status_code()
    } else {
        http::StatusCode::OK
    };

    let body = match &t {
        Ok(ok) => format!("{}", ok),
        Err(err) => format!("{}", err),
    };

    let content_type = match &t {
        Ok(ok) => ok.content_type(),
        Err(_) => "text/plain",
    };

    warp::reply::with_header(
        warp::reply::with_status(body, status_code),
        http::header::CONTENT_TYPE,
        content_type,
    )
}

async fn result_to_response<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

#[derive(Debug)]
struct Hash([u8; 64]);

impl FromStr for Hash {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Hash, crate::Error> {
        Ok(Hash(base64_url::decode(s)?.try_into().map_err(
            |e: Vec<_>| format!("expected 64 bytes; got {}", e.len()),
        )?))
    }
}

pub fn get_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / String)
        .map(|hash: String| async move {
            let Hash(hash) = Hash::from_str(&hash)?;
            let DB.get(&hash);
            Ok(format!("{:?}", Hash::from_str(&hash)?)) as Result<_, crate::Error>
        })
        .and_then(result_to_response)
}
