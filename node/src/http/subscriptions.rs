use serde_derive::Deserialize;
use warp::Filter;

use samizdat_common::Key;

use crate::balanced_or_tree;
use crate::models::{Dropable, Subscription, SubscriptionKind, SubscriptionRef};

use super::{reply, returnable, authenticate};

/// The entrypoint of the subscriptions API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        get_subscription(),
        get_subscriptions(),
        post_subscription(),
        delete_subscription(),
    )
}

/// Creates a new subscription, i.e., a command to listen and react to new edition announcements.
fn post_subscription() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    #[derive(Deserialize)]
    struct Request {
        public_key: String,
        #[serde(default)]
        kind: SubscriptionKind,
    }

    warp::path!("_subscriptions")
        .and(warp::post())
        .and(authenticate())
        .and(warp::body::json())
        .map(|request: Request| {
            let subscription = SubscriptionRef::build(Subscription::new(
                request.public_key.parse()?,
                request.kind,
            ));
            Ok(subscription?.public_key.to_string())
        })
        .map(reply)
}

/// Removes a subscription.
fn delete_subscription(
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_subscriptions" / Key)
        .and(warp::delete())
        .and(authenticate())
        .map(|public_key: Key| {
            let subscription = SubscriptionRef::new(public_key);
            Ok(subscription.drop_if_exists())
        })
        .map(reply)
}

/// Gets information associates with a series owner
fn get_subscription() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_subscriptions" / Key)
        .and(warp::get())
        .and(authenticate())
        .map(|public_key: Key| {
            let maybe_subscription = SubscriptionRef::new(public_key).get()?;
            Ok(returnable::Json(maybe_subscription))
        })
        .map(reply)
}

/// Gets information associates with a series owner
fn get_subscriptions() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_subscriptions")
        .and(warp::get())
        .and(authenticate())
        .map(|| {
            let subscriptions = SubscriptionRef::get_all()?;
            Ok(returnable::Json(subscriptions))
        })
        .map(reply)
}
