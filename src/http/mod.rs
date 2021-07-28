mod returnable;

pub use returnable::Returnable;

use std::convert::TryInto;
use std::str::FromStr;
use warp::reject::Reject;
use warp::Filter;
use std::fmt::{self, Display};
use sha3::{Digest, Sha3_512};

use crate::DB;
use crate::flatbuffers;

impl Reject for crate::Error {}

fn reply<T>(t: Result<T, crate::Error>) -> impl warp::Reply
where
    T: Returnable,
{
    warp::reply::with_header(
        warp::reply::with_status(t.render().into_owned(), t.status_code()),
        http::header::CONTENT_TYPE,
        &*t.content_type(),
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

impl Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(&self.0))
    }
}

impl Hash {
    /// # Panics
    /// 
    /// If the received slice does not have the correct length of 64 bytes.
    fn build(x: impl AsRef<[u8]>) -> Hash {
        Hash(x.as_ref().try_into().expect("bad hash value"))
    }
}

pub fn get_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / String)
        .and(warp::get())
        .map(|hash: String| {
            let Hash(hash) = Hash::from_str(&hash)?;
            let object = DB.get(&hash)?;

            if let Some(object) = &object {
                let object = flatbuffers::object::root_as_object(object)?;
            }

            Ok(object) as Result<_, crate::Error>
        })
}

pub fn post_content() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_content")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::body::bytes())
        .map(|content_type: String, bytes: bytes::Bytes| async move {
            let object = flatbuffers::build_object(&content_type, &*bytes);
            let hash = Hash::build(Sha3_512::digest(&*object));
            DB.put(&hash.0, object)?;
            Ok(hash.to_string())
        })
        .and_then(result_to_response)
}
