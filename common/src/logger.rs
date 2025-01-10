//! Logging configuration module for Samizdat applications.
//!
//! This module provides initialization and configuration of the application's logging system
//! using the tracing framework. It sets up appropriate log levels for different components
//! and configures the logging output format.

use tracing::Level;
use tracing_subscriber::{filter, layer::SubscriberExt, util::SubscriberInitExt};

/// Initializes the logging system with predefined configuration.
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
