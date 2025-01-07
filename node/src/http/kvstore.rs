//! A key-value store on the same line of the `LocalStorage` Web API, but partitioned by
//! identity. This provides better context isolation for Samizdat web applications than
//! `LocalStorage` currently does.

use axum::extract::{DefaultBodyLimit, Path};
use axum::{Json, Router};
use futures::FutureExt;
use samizdat_common::db::{readonly_tx, writable_tx, Table as _};
use serde_derive::Deserialize;

use crate::access::{AccessRight, Entity};
use crate::db::Table;
use crate::security_scope;

use super::auth::SecurityScope;
use super::ApiResponse;

/// The authentication management API.
pub fn api() -> Router {
    Router::new()
        .route(
            "/{*tail}",
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
    let mut key = vec![];

    bincode::serialize_into(&mut key, entity).expect("can serialize");
    key.push(b'\0');
    bincode::serialize_into(&mut key, tail).expect("can serialize");

    key
}

fn prefix(entity: &Entity) -> Vec<u8> {
    let mut prefix = vec![];

    bincode::serialize_into(&mut prefix, entity).expect("can serialize");
    prefix.push(b'\0');

    prefix
}

async fn get(
    Path(tail): Path<String>,
    SecurityScope(entity): SecurityScope,
) -> ApiResponse<Option<String>> {
    async move {
        let maybe_value = readonly_tx(|tx| {
            Table::KVStore.get(tx, key(&entity, &tail), |bytes| {
                String::from_utf8_lossy(bytes).into_owned()
            })
        });
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
        writable_tx(|tx| {
            Table::KVStore.put(tx, key(&entity, &tail), request.value.as_str());
            Ok(())
        })
    }
    .map(ApiResponse)
    .await
}

/// Clears a key in the store.
async fn delete(Path(tail): Path<String>, SecurityScope(entity): SecurityScope) -> ApiResponse<()> {
    async move {
        writable_tx(|tx| {
            Table::KVStore.delete(tx, key(&entity, &tail));
            Ok(())
        })
    }
    .map(ApiResponse)
    .await
}

/// Clears the whole store.
async fn clear(SecurityScope(entity): SecurityScope) -> ApiResponse<()> {
    async move {
        writable_tx(|tx| {
            Table::KVStore.prefix(prefix(&entity)).delete(tx);
            Ok(())
        })
    }
    .map(ApiResponse)
    .await
}
