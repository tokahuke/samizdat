//! Subscriptions API.

use axum::extract::Path;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures::FutureExt;
use samizdat_common::db::{readonly_tx, writable_tx, Droppable};
use serde_derive::Deserialize;

use samizdat_common::Key;

use crate::access::AccessRight;
use crate::http::ApiResponse;
use crate::models::{Subscription, SubscriptionKind, SubscriptionRef};
use crate::security_scope;

/// The entrypoint of the subscriptions API.
pub fn api() -> Router {
    #[derive(Deserialize)]
    struct PostSubscriptionRequest {
        public_key: String,
        #[serde(default)]
        kind: SubscriptionKind,
    }

    Router::new()
        .route(
            // Creates a new subscription, i.e., a command to listen and react to new edition
            // announcements.
            "/",
            post(|Json(request): Json<PostSubscriptionRequest>| {
                async move {
                    let subscription = writable_tx(|tx| {
                        SubscriptionRef::build(
                            tx,
                            Subscription::new(request.public_key.parse()?, request.kind),
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
            "/:key/refresh",
            get(|Path(public_key): Path<Key>| {
                async move {
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
            // Removes a subscription.
            "/:key",
            delete(|Path(public_key): Path<Key>| {
                async move {
                    let subscription = SubscriptionRef::new(public_key);
                    let existed = readonly_tx(|tx| subscription.get(tx))?.is_some();
                    subscription.drop_if_exists()?;
                    Ok(existed)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
        .route(
            // Gets information associates with a series owner
            "/:key",
            get(|Path(public_key): Path<Key>| {
                async move {
                    let maybe_subscription =
                        readonly_tx(|tx| SubscriptionRef::new(public_key).get(tx))?;
                    Ok(maybe_subscription)
                }
                .map(ApiResponse)
            })
            .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
        .route(
            "/",
            get(|| async move { readonly_tx(|tx| SubscriptionRef::get_all(tx)) }.map(ApiResponse))
                .layer(security_scope!(AccessRight::ManageSubscriptions)),
        )
}
