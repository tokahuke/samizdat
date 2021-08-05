mod returnable;

pub use returnable::{Return, Returnable};

use std::str::FromStr;
use warp::Filter;

use samizdat_common::Hash;

use crate::db::Table;
//use crate::flatbuffers;
use crate::object::Object;
use crate::{db, hubs};

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

async fn async_reply<F, T>(fut: F) -> Result<Box<dyn warp::Reply>, warp::Rejection>
where
    F: std::future::Future<Output = Result<T, crate::Error>>,
    T: 'static + Returnable,
{
    Ok(Box::new(reply(fut.await)) as Box<dyn warp::Reply>)
}

pub fn get_hash() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_hash" / String)
        .and(warp::get())
        .map(|hash: String| async move {
            // Try get from local:
            let hash = Hash::from_str(&hash)?;
            let serialized = db().get_cf(Table::Content.get(), &hash)?;

            // Else, fallback to get from peers:
            let serialized = if let Some(serialized) = serialized {
                log::info!("found local hash {}", hash);
                Some(serialized)
            } else {
                hubs().query(hash).await
            };

            // Respond with found or not found.
            if let Some(serialized) = serialized {
                let object: Object = bincode::deserialize(&serialized)?;

                Ok(Some(Return {
                    content_type: object.content_type.to_owned(),
                    status_code: http::StatusCode::OK,
                    // TODO: DANGER! double copy of large, large content!!
                    content: object.content.to_owned(),
                }))
            } else {
                Ok(None)
            }
        })
        .and_then(async_reply)
}

pub fn post_content() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("_content")
        .and(warp::post())
        .and(warp::header("content-type"))
        .and(warp::body::bytes())
        .map(|content_type: String, bytes: bytes::Bytes| {
            let object = Object::new(&content_type, &*bytes);
            let serialized = bincode::serialize(&object).expect("can serialize");
            let hash = Hash::build(&serialized);

            let mut batch = rocksdb::WriteBatch::default();
            batch.put_cf(Table::Hashes.get(), &hash, &[]);
            batch.put_cf(Table::Content.get(), &hash, serialized);
            db().write(batch)?;

            Ok(hash.to_string())
        })
        .map(reply)
}
