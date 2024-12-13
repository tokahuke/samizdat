//! A key-value store on the same line of the `LocalStorage` Web API, but partitioned by
//! identity. This provides better context isolation for Samizdat web applications than
//! `LocalStorage` currently does.

use axum::extract::{DefaultBodyLimit, Path};
use axum::{Json, Router};
use futures::FutureExt;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::Deserialize;

use crate::access::{AccessRight, Entity};
use crate::db::{db, Table};
use crate::security_scope;

use super::auth::SecurityScope;
use super::ApiResponse;

/// The authentication management API.
pub fn api() -> Router {
    Router::new()
        .route(
            "/*tail",
            axum::routing::get(get)
                .layer(security_scope!(AccessRight::Public))
                .put(put)
                .layer(
                    tower::ServiceBuilder::new()
                        .layer(security_scope!(AccessRight::Public))
                        .layer(DefaultBodyLimit::disable()),
                )
                .delete(delete)
                .layer(security_scope!(AccessRight::Public)),
        )
        .route(
            "/",
            axum::routing::delete(clear).layer(security_scope!(AccessRight::Public)),
        )
}

/// The full prefix of the keys to be stored in RocksDB. This provides the desired context
/// isolation.
fn key(entity: &Entity, tail: &str) -> Vec<u8> {
    bincode::serialize(&(entity, tail)).expect("can serialize")
}

async fn get(
    Path(tail): Path<String>,
    SecurityScope(entity): SecurityScope,
) -> ApiResponse<Option<String>> {
    async move {
        let maybe_value_encoded = db().get_cf(Table::KVStore.get(), key(&entity, &tail))?;
        let maybe_value =
            maybe_value_encoded.map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

        Ok(maybe_value)
    }
    .map(ApiResponse)
    .await
}

#[derive(Deserialize)]
struct PutRequest {
    value: String,
}

/// Inserts a value for a key in the store.
#[axum::debug_handler]
async fn put(
    Path(tail): Path<String>,
    SecurityScope(entity): SecurityScope,
    Json(request): Json<PutRequest>,
) -> ApiResponse<()> {
    async move {
        db().put_cf(
            Table::KVStore.get(),
            key(&entity, &tail),
            request.value.as_bytes(),
        )?;
        Ok(())
    }
    .map(ApiResponse)
    .await
}

/// Clears a key in the store.
async fn delete(Path(tail): Path<String>, SecurityScope(entity): SecurityScope) -> ApiResponse<()> {
    async move {
        db().delete_cf(Table::KVStore.get(), key(&entity, &tail))?;
        Ok(())
    }
    .map(ApiResponse)
    .await
}

/// Clears the whole store.
async fn clear(SecurityScope(entity): SecurityScope) -> ApiResponse<()> {
    async move {
        let mut batch = WriteBatch::default();

        for item in db().iterator_cf(Table::KVStore.get(), IteratorMode::Start) {
            let (key, _) = item?;
            let (key_entity, _): (Entity, String) = bincode::deserialize(&key)?;
            if entity == key_entity {
                batch.delete_cf(Table::KVStore.get(), &key);
            }
        }

        db().write(batch)?;

        Ok(())
    }
    .map(ApiResponse)
    .await
}
