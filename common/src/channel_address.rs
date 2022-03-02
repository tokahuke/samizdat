//! A channel is a sub-division of a QUIC connection. Channels are used in
//! connections between peers to enable them to keep simultaneous requests
//! in the same connection. Remember that we cannot create connections from,
//! e.g., ephemeral ports because NATs/Firewalls.
//!

use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Debug, Display};
use std::net::SocketAddr;

use crate::Hash;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ChannelAddr {
    peer_addr: SocketAddr,
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
    pub fn new(peer_addr: SocketAddr, channel_id: u32) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id,
        }
    }

    pub fn from_socket_and_hash(peer_addr: SocketAddr, hash: Hash) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id: u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]),
        }
    }

    pub fn channel_id(&self) -> u32 {
        self.channel_id
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
}
