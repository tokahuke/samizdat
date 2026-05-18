//! Provides address-related types and functionality for the Samizdat network.
//!
//! This module contains definitions for channel addresses, socket addresses, and IP address
//! resolution modes used in peer-to-peer communication.

use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Debug, Display};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use crate::Hash;

/// Represents a channel for a multiplexed QUIC connection.
///
/// Widened to 64 bits so randomly-assigned channel ids have a negligible birthday-bound
/// collision probability: ~50% at ~4 billion concurrent ids, vs. ~65k for the previous
/// 32-bit version. Collisions in the 32-bit form were a remote-triggerable panic via the
/// asserts in `Matcher::expect/arrive`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChannelId(u64);

impl From<u64> for ChannelId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<ChannelId> for u64 {
    fn from(value: ChannelId) -> Self {
        value.0
    }
}

impl ChannelId {
    /// Allocates a cryptographically random channel id.
    pub fn random() -> Self {
        let mut buf = [0u8; 8];
        getrandom::getrandom(&mut buf).expect("getrandom failed");
        Self(u64::from_le_bytes(buf))
    }

    /// Encodes the id as 8 big-endian bytes, suitable for sending on a wire.
    pub fn to_be_bytes(self) -> [u8; 8] {
        self.0.to_be_bytes()
    }

    /// Decodes the id from 8 big-endian bytes.
    pub fn from_be_bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_be_bytes(bytes))
    }
}

impl fmt::Debug for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}
impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// A channel is a sub-division of a QUIC connection. Channels are used in connections
/// between peers to enable them to keep simultaneous requests in the same connection.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ChannelAddr {
    /// The socket address for this channel address.
    peer_addr: SocketAddr,
    /// The specific channel identifier within the connection.
    channel_id: ChannelId,
}

impl Display for ChannelAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.peer_addr, self.channel_id,)
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
    pub fn new(peer_addr: SocketAddr, channel_id: ChannelId) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id,
        }
    }

    /// Derives a special channel address from a given hash value. Uses the first 8 bytes
    /// of the hash as the channel id.
    pub fn from_socket_and_hash(peer_addr: SocketAddr, hash: Hash) -> ChannelAddr {
        let mut id_bytes = [0u8; 8];
        id_bytes.copy_from_slice(&hash[..8]);
        ChannelAddr {
            peer_addr,
            channel_id: ChannelId::from_be_bytes(id_bytes),
        }
    }

    /// The channel id for this channel address.
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// The socket address for this channel address.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
}

/// Specifies how DNS resolution should handle IPv4 and IPv6 addresses.
///
/// Controls the preference and filtering of IP address types when resolving hostnames,
/// allowing for explicit version requirements or flexible fallback behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AddrResolutionMode {
    /// Only use IPv6 addresses and ignore IPv4 entries.
    EnsureIpv6,
    /// Only use IPv4 addresses and ignore IPv6 entries.
    EnsureIpv4,
    /// Use IPv6 addresses when possible, but default to an IPv4 if necessary.
    PreferIpv6,
    /// Use IPv4 addresses when possible, but default to an IPv6 if necessary.
    PreferIpv4,
    /// Use both IPv6 and IPv4 addresses. If both are present, two addresses will be
    /// resolved for the same name.
    UseBoth,
}

impl FromStr for AddrResolutionMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ensure-ipv4" => Ok(Self::EnsureIpv4),
            "ensure-ipv6" => Ok(Self::EnsureIpv6),
            "prefer-ipv4" => Ok(Self::PreferIpv4),
            "prefer-ipv6" => Ok(Self::PreferIpv6),
            "use-both" => Ok(Self::UseBoth),
            invalid => Err(format!("Invalid address resolution mode `{invalid}`")),
        }
    }
}

impl AddrResolutionMode {
    /// Choose from a list of addresses which ones to use.
    fn filter_hosts(self, hosts: &[SocketAddr]) -> Vec<SocketAddr> {
        // Iterator factory (makes IPs canonical).
        let iter_hosts = || {
            hosts
                .iter()
                .map(|addrs| SocketAddr::new(addrs.ip().to_canonical(), addrs.port()))
        };

        match self {
            Self::EnsureIpv6 => iter_hosts()
                .filter(|addr| addr.ip().is_ipv6())
                .take(1)
                .collect(),
            Self::EnsureIpv4 => iter_hosts()
                .filter(|addr| addr.ip().is_ipv4())
                .take(1)
                .collect(),
            Self::PreferIpv6 => iter_hosts()
                .max_by_key(|addr| if addr.ip().is_ipv6() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::PreferIpv4 => iter_hosts()
                .max_by_key(|addr| if addr.ip().is_ipv4() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::UseBoth => {
                let an_ipv6 = iter_hosts().filter(|addr| addr.ip().is_ipv6()).take(1);
                // Loopbacks are coerced to IPv6.
                let an_ipv4 = iter_hosts()
                    .filter(|addr| addr.ip().is_ipv4())
                    .filter(|addr| !addr.ip().is_loopback())
                    .take(1);

                an_ipv4.chain(an_ipv6).collect()
            }
        }
    }

    pub async fn resolve(&self, host: &str) -> Result<Vec<(String, SocketAddr)>, crate::Error> {
        if let Ok(socket) = host.parse::<SocketAddr>() {
            return Ok(vec![(host.to_owned(), socket)]);
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![(host.to_owned(), SocketAddr::new(ip, 4511))]);
        }

        // Now we know it's not a socket or IP address, we can do this safely:
        // (remember: IPv6 also have `:`)
        let (domain, port_str) = host.rsplit_once(':').unwrap_or((host, "4511"));
        let port: u16 = port_str
            .parse()
            .map_err(|err| format!("bad host name {host}: {err}"))?;

        Ok(self
            .filter_hosts(
                &tokio::net::lookup_host((domain, port))
                    .await?
                    .collect::<Vec<_>>(),
            )
            .into_iter()
            .map(|addrs| (host.to_owned(), addrs))
            .collect::<Vec<_>>())
    }
}
