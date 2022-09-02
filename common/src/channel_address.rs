//! A channel is a sub-division of a QUIC connection. Channels are used in
//! connections between peers to enable them to keep simultaneous requests
//! in the same connection. Remember that we cannot create connections from,
//! e.g., ephemeral ports because NATs/Firewalls.

use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Debug, Display};
use std::net::SocketAddr;

use crate::Hash;

/// A channel is a sub-division of a QUIC connection. This serves to multiplex a costly
/// QUIC connection in many cheap ephemeral sub-connections.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ChannelAddr {
    /// The socket address for this channel address.
    peer_addr: SocketAddr,
    /// The channel id for this channel address.
    channel_id: u32,
}

impl Display for ChannelAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{:x}", self.peer_addr, self.channel_id,)
    }
}

impl Debug for ChannelAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl ChannelAddr {
    /// Creates a new channel address from a given socket address (IP+port) and an
    /// identifier for this specific channel.
    pub fn new(peer_addr: SocketAddr, channel_id: u32) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id,
        }
    }

    /// Derives a special channel address from a given hash value.
    pub fn from_socket_and_hash(peer_addr: SocketAddr, hash: Hash) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id: u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]),
        }
    }

    /// The channel id for this channel address.
    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }

    /// The socket address for this channel address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
}
