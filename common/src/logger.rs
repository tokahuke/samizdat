//! Tracing setup shared by every Samizdat binary. Picks per-target log
//! levels and the output format.

use tracing::Level;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

/// Install the tracing subscriber. Call once per process.
pub fn init() {
    tracing_subscriber::registry()
        .with(
            filter::Targets::new()
                .with_default(Level::INFO)
                .with_target("tower_http::trace", Level::DEBUG)
                .with_target("tarpc", Level::WARN),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}
