//! Underlying infrastructure over QUIC for communication in the Samizdat network.

mod channel_manager;
mod connection_manager;
mod matcher;
mod multiplexed;

pub use self::channel_manager::{ChannelManager, ChannelReceiver, ChannelSender};
pub use self::connection_manager::ConnectionManager;
