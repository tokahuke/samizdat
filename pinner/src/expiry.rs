//! Periodic loop that drops subscriptions whose paid expiry has passed.
//!
//! Tickless backoff: if the node admin call fails for a given key, the row
//! stays and the next tick retries. Pinner restart is safe -- every tick
//! reads fresh state from LMDB, nothing is in-memory.

use std::time::Duration;

use chrono::Utc;

use crate::cli::cli;
use crate::{db, node_client};

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
        tracing::info!("expiring pin for {key}");
        if let Err(err) = node_client::get().drop_subscription(&key).await {
            tracing::warn!("dropping subscription for {key} failed: {err}; will retry next tick");
            continue;
        }
        if let Err(err) = db::delete(&key) {
            tracing::warn!("local db delete for expired {key} failed: {err}");
        }
    }
    Ok(())
}
