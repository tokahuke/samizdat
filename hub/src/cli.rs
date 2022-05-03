use std::fmt::{self, Display};
use std::net::SocketAddr;
use std::num::ParseIntError;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Cli {
    /// Set logging level.
    #[structopt(env = "SAMIZDAT_VERBOSE", long, short = "v")]
    pub verbose: bool,
    /// The socket addresses for nodes to connect as clients.
    #[structopt(env = "SAMIZDAT_DIRECT_ADDRESSES", long, default_value = "[::]:4511")]
    pub direct_addresses: Vec<SocketAddr>,
    /// The port for nodes to connect as servers.
    #[structopt(env = "SAMIZDAT_REVERSE_ADDRESSES", long, default_value = "[::]:4512")]
    pub reverse_addresses: Vec<SocketAddr>,
    #[structopt(env = "SAMIZDAT_DATA", long, default_value = "data/hub")]
    pub data: String,
    /// Maximum number of simultaneous connections.
    #[structopt(env = "SAMIZDAT_MAX_CONNECTIONS", long, default_value = "2048")]
    pub max_connections: usize,
    /// Maximum number of _simultaneous_ resolutions per query.
    #[structopt(env = "SAMIZDAT_MAX_RESOLUTIONS_PER_QUERY", long, default_value = "12")]
    pub max_resolutions_per_query: usize,
    /// The maximum number of _simultaneous_ requests a node can make.
    #[structopt(env = "SAMIZDAT_MAX_QUERY_PER_NODE", long, default_value = "12")]
    pub max_queries_per_node: usize,
    /// The inverse of the interval that we delay if a node is requesting too many queries.
    /// (e.g., 2 => delay 500ms).
    #[structopt(env = "SAMIZDAT_MAX_QUERY_RATE_PER_NODE", long, default_value = "12")]
    pub max_query_rate_per_node: f64,
    /// The maximum number of candidates to return to the client.
    #[structopt(env = "SAMIZDAT_MAX_CANDIDATES", long, default_value = "3")]
    pub max_candidates: usize,
    /// Other servers to which to listen to.
    #[structopt(env = "SAMIZDAT_PARTNERS", long)]
    pub partners: Option<Vec<AddrToResolve>>,
    /// The port for the monitoring http server.
    #[structopt(env = "SAMIZDAT_HTTP_PORT", long, default_value = "45180")]
    pub http_port: u16,
}

/// A flexible representation of an address in the internet.
#[derive(Debug, Clone)]
pub enum AddrToResolve {
    /// A raw `ip:port` address.
    SocketAddr(SocketAddr),
    /// A `domain:port` (or `domain`, only) address.
    DomainAndPort(String, u16),
}

impl FromStr for AddrToResolve {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, ParseIntError> {
        if let Ok(socket_addr) = s.parse::<SocketAddr>() {
            Ok(AddrToResolve::SocketAddr(socket_addr))
        } else if let Some(pos) = s.find(':') {
            Ok(AddrToResolve::DomainAndPort(
                s[0..pos].to_owned(),
                s[pos + 1..].parse::<u16>()?,
            ))
        } else {
            Ok(AddrToResolve::DomainAndPort(s.to_owned(), 4511))
        }
    }
}

impl Display for AddrToResolve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddrToResolve::SocketAddr(addr) => write!(f, "{addr}"),
            AddrToResolve::DomainAndPort(domain, port) if *port == 4511 => write!(f, "{domain}"),
            AddrToResolve::DomainAndPort(domain, port) => write!(f, "{}:{}", domain, port),
        }
    }
}

impl AddrToResolve {
    fn name(&self) -> String {
        match self {
            AddrToResolve::SocketAddr(addr) => addr.to_string(),
            AddrToResolve::DomainAndPort(domain, port) if *port == 4511 => domain.to_owned(),
            AddrToResolve::DomainAndPort(domain, port) => format!("{}:{}", domain, port),
        }
    }

    pub async fn resolve(&self) -> Result<(&'static str, SocketAddr), crate::Error> {
        fn prefer_ipv6(it: impl IntoIterator<Item = SocketAddr>) -> Option<SocketAddr> {
            it.into_iter()
                .max_by_key(|addr| if addr.is_ipv6() { 1 } else { 0 })
        }

        let name = Box::leak(self.name().into_boxed_str());
        let addr = match self {
            AddrToResolve::SocketAddr(addr) => *addr,
            AddrToResolve::DomainAndPort(domain, port) => {
                prefer_ipv6(tokio::net::lookup_host((&**domain, *port)).await?)
                    .ok_or_else(|| format!("no such host {}", domain))?
            }
        };

        Ok((name, addr))
    }
}
