use tracing::Level;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

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
