use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::Deserialize;
use warp::Filter;

use crate::access::Entity;
use crate::balanced_or_tree;
use crate::db::{db, Table};

use super::{api_reply, auth};

/// The authentication management API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(get(), put(), delete(), clear(),)
}

fn key(entity: &Entity, tail: &warp::path::Tail) -> Vec<u8> {
    bincode::serialize(&(entity, tail.as_str())).expect("can serialize")
}

pub fn get() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_kvstore" / ..)
        .and(warp::get())
        .and(warp::path::tail())
        .and(auth::security_scope())
        .map(|tail, entity: Entity| {
            let maybe_value_encoded = db().get_cf(Table::KVStore.get(), key(&entity, &tail))?;
            let maybe_value =
                maybe_value_encoded.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

            Ok(maybe_value)
        })
        .map(api_reply)
}

pub fn put() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        value: String,
    }

    warp::path!("_kvstore" / ..)
        .and(warp::put())
        .and(warp::path::tail())
        .and(auth::security_scope())
        .and(warp::body::content_length_limit(8_192))
        .and(warp::body::json())
        .map(|tail, entity: Entity, request: Request| {
            db().put_cf(
                Table::KVStore.get(),
                key(&entity, &tail),
                request.value.as_bytes(),
            )?;
            Ok(())
        })
        .map(api_reply)
}

pub fn delete() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_kvstore" / ..)
        .and(warp::delete())
        .and(warp::path::tail())
        .and(auth::security_scope())
        .map(|tail, entity: Entity| {
            db().delete_cf(Table::KVStore.get(), key(&entity, &tail))?;
            Ok(())
        })
        .map(api_reply)
}

pub fn clear() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_kvstore")
        .and(warp::delete())
        .and(auth::security_scope())
        .map(|entity: Entity| {
            let mut batch = WriteBatch::default();

            for (key, _) in db().iterator_cf(Table::KVStore.get(), IteratorMode::Start) {
                let (key_entity, _): (Entity, String) = bincode::deserialize(&*key)?;
                if entity == key_entity {
                    batch.delete_cf(Table::KVStore.get(), &key);
                }
            }

            db().write(batch)?;

            Ok(())
        })
        .map(api_reply)
}
