//! Underlying infrastructure over QUIC for communication in the Samizdat network.

pub mod channel_manager;
pub mod file_transfer;

mod connection_manager;
mod matcher;
mod multiplexed;

pub use self::channel_manager::{ChannelReceiver, ChannelSender, PEER_CONNECTIONS};
pub use self::connection_manager::connection_manager;
