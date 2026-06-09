//! Periodic loop that drops subscriptions whose paid expiry has passed.
//!
//! Tickless backoff: if the node admin call fails for a given key, the row
//! stays and the next tick retries. Pinner restart is safe -- every tick
//! reads fresh state from LMDB, nothing is in-memory.

use std::time::Duration;

use chrono::Utc;

use crate::{cli::cli, db, node_client};

pub async fn run() {
    let tick = Duration::from_secs(cli().expiry_tick_seconds);
    tracing::info!("expiry loop running every {}s", tick.as_secs());

    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        interval.tick().await;
        if let Err(err) = sweep().await {
            tracing::error!("expiry sweep failed: {err}");
        }
    }
}

async fn sweep() -> Result<(), anyhow::Error> {
    let now = Utc::now();
    let expired = db::list_expired(now)?;
    for key in expired {
        // Atomic re-check + delete inside a single writable_tx. If a
        // customer renewed via `POST /pin` between the list_expired read
        // and now, `delete_if_expired` returns Ok(false) and we leave
        // the row alone.
        match db::delete_if_expired(&key, now) {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!("pin for {key} was renewed between list and sweep; skipping");
                continue;
            }
            Err(err) => {
                tracing::warn!("local db delete_if_expired for {key} failed: {err}");
                continue;
            }
        }
        // DB-first ordering: we have already cleared our side. The node
        // call is best-effort; if it fails the operator gets an orphaned
        // subscription on the node (storage isn't freed) but no paid
        // user loses content. Reverse ordering would risk the bigger
        // bug -- dropping a renewed pin if the node call succeeded then
        // the in-tx check rejected.
        tracing::info!("expired pin for {key}; releasing node subscription");
        if let Err(err) = node_client::get().drop_subscription(&key).await {
            tracing::warn!(
                "node drop_subscription({key}) failed after local delete: {err}; \
                 the subscription is now orphaned on the node and storage will not be \
                 reclaimed until a manual cleanup or the node is restarted"
            );
        }
    }
    Ok(())
}
