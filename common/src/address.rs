//! Definitions for addresses relevant to Samizdat.

use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Debug, Display};
use std::net::{IpAddr, SocketAddr};
use std::num::ParseIntError;
use std::str::FromStr;

use crate::Hash;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChannelId(u32);

impl From<u32> for ChannelId {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<ChannelId> for u32 {
    fn from(value: ChannelId) -> Self {
        value.0
    }
}

impl fmt::Debug for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}
impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

/// A channel is a sub-division of a QUIC connection. Channels are used in connections
/// between peers to enable them to keep simultaneous requests in the same connection.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ChannelAddr {
    /// The socket address for this channel address.
    peer_addr: SocketAddr,
    /// The channel id for this channel address.
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

    /// Derives a special channel address from a given hash value.
    pub fn from_socket_and_hash(peer_addr: SocketAddr, hash: Hash) -> ChannelAddr {
        ChannelAddr {
            peer_addr,
            channel_id: u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]).into(),
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

/// A representation of a hub location.
#[derive(Clone, Copy)]
pub struct HubAddr {
    /// The IP address of the hub.
    ip: IpAddr,
    /// The port of the node-to-hub RPC.
    direct_port: u16,
    /// The port of the hub-to-node RPC.
    reverse_port: u16,
}

impl FromStr for HubAddr {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(pos) = s.find('/') {
            let addr = s[..pos]
                .parse::<SocketAddr>()
                .map_err(|err| err.to_string())?;
            Ok(HubAddr {
                ip: addr.ip(),
                direct_port: addr.port(),
                reverse_port: s[pos + 1..].parse::<u16>().map_err(|err| err.to_string())?,
            })
        } else {
            let addr = s.parse::<SocketAddr>().map_err(|err| err.to_string())?;
            Ok(HubAddr {
                ip: addr.ip(),
                direct_port: addr.port(),
                reverse_port: addr.port() + 1,
            })
        }
    }
}
impl From<SocketAddr> for HubAddr {
    fn from(addr: SocketAddr) -> Self {
        HubAddr {
            ip: addr.ip(),
            direct_port: addr.port(),
            reverse_port: addr.port() + 1,
        }
    }
}

impl Display for HubAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.reverse_port == self.direct_port + 1 {
            write!(f, "{}", SocketAddr::from((self.ip, self.direct_port)))
        } else {
            write!(
                f,
                "{}/{}",
                SocketAddr::from((self.ip, self.direct_port)),
                self.reverse_port
            )
        }
    }
}

impl Debug for HubAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as Display>::fmt(&self, f)
    }
}

impl HubAddr {
    /// Create a new [`HubAddr`] from a [`SocketAddr`] and a reverse port (for the hub-to-node RPC).
    pub fn new(addr: SocketAddr, reverse_port: u16) -> Self {
        HubAddr {
            ip: addr.ip(),
            direct_port: addr.port(),
            reverse_port,
        }
    }

    /// Makes the IP of this [`HubAddr`] canonical.
    pub fn to_canonical(&self) -> HubAddr {
        HubAddr {
            ip: self.ip.to_canonical(),
            direct_port: self.direct_port,
            reverse_port: self.reverse_port,
        }
    }

    /// The full socket address of the node-to-hub RPC.
    pub fn direct_addr(&self) -> SocketAddr {
        (self.ip, self.direct_port).into()
    }

    /// The full socket address of the hub-to-node RPC.
    pub fn reverse_addr(&self) -> SocketAddr {
        (self.ip, self.reverse_port).into()
    }
}

/// Represents either an `ip:port` style address or a `domain:port` style address. This
/// is intended to be a flexible representation of an address in the internet.
#[derive(Debug)]
pub enum SocketOrDomain {
    /// A raw `ip:port` address.
    SocketAddr(SocketAddr),
    /// A `domain:port` (or `domain`, only) address.
    DomainAndPort(String, u16),
}

impl FromStr for SocketOrDomain {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, ParseIntError> {
        if let Ok(socket_addr) = s.parse::<SocketAddr>() {
            Ok(SocketOrDomain::SocketAddr(socket_addr))
        } else if let Some(pos) = s.find(':') {
            Ok(SocketOrDomain::DomainAndPort(
                s[0..pos].to_owned(),
                s[pos + 1..].parse::<u16>()?,
            ))
        } else {
            Ok(SocketOrDomain::DomainAndPort(s.to_owned(), 4511))
        }
    }
}

impl SocketOrDomain {
    /// The port of this address.
    fn port(&self) -> u16 {
        match self {
            SocketOrDomain::SocketAddr(addr) => addr.port(),
            SocketOrDomain::DomainAndPort(_, port) => *port,
        }
    }
}

impl Display for SocketOrDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SocketOrDomain::SocketAddr(addr) => write!(f, "{addr}"),
            SocketOrDomain::DomainAndPort(domain, port) if *port == 4511 => write!(f, "{domain}"),
            SocketOrDomain::DomainAndPort(domain, port) => write!(f, "{domain}:{port}"),
        }
    }
}

/// A representation of a double-port address (linking to a Samizdat hub).
#[derive(Debug)]
pub struct AddrToResolve {
    /// The address of the node-to-hub RPC.
    direct_addr: SocketOrDomain,
    /// The port of the hub-to-node RPC. The IP of this RPC is always the same as of
    /// `direct_addr`.
    reverse_port: u16,
}

impl FromStr for AddrToResolve {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, ParseIntError> {
        if let Some(pos) = s.find('/') {
            let direct_addr = s[..pos].parse::<SocketOrDomain>()?;
            Ok(AddrToResolve {
                direct_addr,
                reverse_port: s[pos + 1..].parse::<u16>()?,
            })
        } else {
            let direct_addr = s.parse::<SocketOrDomain>()?;
            Ok(AddrToResolve {
                reverse_port: direct_addr.port() + 1,
                direct_addr,
            })
        }
    }
}

impl Display for AddrToResolve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.reverse_port == self.direct_addr.port() + 1 {
            write!(f, "{}", self.direct_addr)
        } else {
            write!(f, "{}/{}", self.direct_addr, self.reverse_port)
        }
    }
}

impl AddrToResolve {
    /// Resolve this address into an iterator of socket addresses.
    pub async fn resolve(
        &self,
        resolution_mode: AddrResolutionMode,
    ) -> Result<impl Iterator<Item = (String, HubAddr)>, crate::Error> {
        let name = self.to_string();

        let addrs = match &self.direct_addr {
            SocketOrDomain::SocketAddr(addr) => vec![HubAddr::new(*addr, self.reverse_port)],
            SocketOrDomain::DomainAndPort(domain, port) => {
                let hosts = resolution_mode.filter_hosts(
                    &tokio::net::lookup_host((&**domain, *port))
                        .await?
                        .map(|addr| HubAddr::new(addr, self.reverse_port))
                        .collect::<Vec<_>>(),
                );

                if hosts.is_empty() {
                    return Err(format!("no such host {}", domain).into());
                } else {
                    hosts
                }
            }
        };

        Ok(addrs.into_iter().map(move |addr| (name.clone(), addr)))
    }
}

/// The way addresses are resolved from DNS.
#[derive(Debug, Clone, Copy)]
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
    fn filter_hosts(self, hosts: &[HubAddr]) -> Vec<HubAddr> {
        // Iterator factory (makes IPs canonical).
        let iter_hosts = || hosts.iter().map(HubAddr::to_canonical);

        match self {
            Self::EnsureIpv6 => iter_hosts()
                .filter(|addr| addr.ip.is_ipv6())
                .take(1)
                .collect(),
            Self::EnsureIpv4 => iter_hosts()
                .filter(|addr| addr.ip.is_ipv4())
                .take(1)
                .collect(),
            Self::PreferIpv6 => iter_hosts()
                .max_by_key(|addr| if addr.ip.is_ipv6() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::PreferIpv4 => iter_hosts()
                .max_by_key(|addr| if addr.ip.is_ipv4() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::UseBoth => {
                let an_ipv6 = iter_hosts().filter(|addr| addr.ip.is_ipv6()).take(1);
                // Loopbacks are coerced to IPv6.
                let an_ipv4 = iter_hosts()
                    .filter(|addr| addr.ip.is_ipv4())
                    .filter(|addr| !addr.ip.is_loopback())
                    .take(1);

                an_ipv4.chain(an_ipv6).collect()
            }
        }
    }
}
