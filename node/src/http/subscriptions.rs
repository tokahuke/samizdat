//! Subscriptions API.

use axum::{
    Json, Router,
    extract::Path,
    routing::{delete, get, post},
};
use futures::FutureExt;
use samizdat_common::db::{Droppable, readonly_tx, writable_tx};
use serde_derive::Deserialize;

use samizdat_common::Key;

use crate::{
    access::AccessRight,
    http::ApiResponse,
    models::{BookmarkType, SeriesRef, Subscription, SubscriptionKind, SubscriptionRef},
    security_scope,
};

/// The entrypoint of the subscriptions API.
pub fn api() -> Router {
    #[derive(Deserialize)]
    struct PostSubscriptionRequest {
        public_key: String,
        #[serde(default)]
        kind: SubscriptionKind,
        /// Cap on total bytes the subscribed series's current edition
        /// may occupy on this node, expressed in megabytes. `None`
        /// falls back to the node's `default_max_edition_size_mb`
        /// config. The node converts to bytes internally; the wire
        /// stays in human units.
        #[serde(default)]
        max_size_mb: Option<u64>,
    }

    Router::new()
        .route(
            // Creates a new subscription, i.e., a command to listen and react to new edition
            // announcements.
            "/",
            post(|Json(request): Json<PostSubscriptionRequest>| {
                async move {
                    let max_bytes = request.max_size_mb.map(|mb| mb.saturating_mul(1_000_000));
                    let subscription = writable_tx(|tx| {
                        SubscriptionRef::build(
                            tx,
                            Subscription::new(request.public_key.parse()?, request.kind, max_bytes),
                        )
                    });
                    Ok(subscription?.public_key.to_string())
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
        .route(
            // Triggers a manual refresh on a subscription.
            "/{key}/refresh",
            get(|Path(public_key): Path<String>| {
                async move {
                    let public_key: Key = public_key.parse()?;
                    let subscription_ref = SubscriptionRef::new(public_key);

                    if readonly_tx(|tx| subscription_ref.exists(tx))? {
                        subscription_ref.trigger_manual_refresh();
                        Ok(())
                    } else {
                        Err(format!("Node is not subscribed to {subscription_ref}").into())
                    }
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
        .route(
            // Removes a subscription. Atomically releases the Reference
            // bookmarks `SeriesRef::advance` placed on the subscribed
            // series's last edition; without this the vacuum cannot
            // reclaim the storage after an unsubscribe.
            "/{key}",
            delete(|Path(public_key): Path<String>| {
                async move {
                    let public_key: Key = public_key.parse()?;
                    let subscription = SubscriptionRef::new(public_key.clone());
                    let series = SeriesRef::new(public_key);

                    let existed = writable_tx(|tx| {
                        let existed = subscription.exists(tx)?;
                        if !existed {
                            return Ok(false);
                        }

                        if let Some(edition) = series.get_last_edition(tx)? {
                            // The Vec is forced: `list_objects` borrows `tx`
                            // immutably for the iterator while
                            // `.bookmark(...).unmark(tx)` needs &mut tx.
                            // Collecting first releases the immutable borrow.
                            let collection = edition.collection();
                            let objects: Vec<_> = collection.list_objects(tx)?.collect();
                            for object in objects {
                                object?.bookmark(BookmarkType::Reference).unmark(tx)?;
                            }
                        }

                        subscription.drop_if_exists_with(tx)?;
                        Ok(true)
                    })?;

                    Ok(existed)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
        .route(
            // Gets information associates with a series owner
            "/{key}",
            get(|Path(public_key): Path<String>| {
                async move {
                    let public_key: Key = public_key.parse()?;
                    let maybe_subscription =
                        readonly_tx(|tx| SubscriptionRef::new(public_key).get(tx))?;
                    Ok(maybe_subscription)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(read; AccessRight::ManageSubscriptions)),
        )
        .route(
            "/",
            get(|| async move { readonly_tx(|tx| SubscriptionRef::get_all(tx)) }.map(ApiResponse))
                .layer(security_scope!(read; AccessRight::ManageSubscriptions)),
        )
}
