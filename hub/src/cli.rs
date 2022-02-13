use std::fmt::{self, Display};
use std::net::{IpAddr, SocketAddr};
use std::num::ParseIntError;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Cli {
    /// The IP to which to bind.
    #[structopt(env = "SAMIZDAT_ADDRESS", long, default_value = "::")]
    pub address: IpAddr,
    /// The port for nodes to connect as clients.
    #[structopt(env = "SAMIZDAT_DIRECT_PORT", long, default_value = "4511")]
    pub direct_port: u16,
    /// The port for nodes to connect as servers.
    #[structopt(env = "SAMIZDAT_REVERSE_PORT", long, default_value = "4512")]
    pub reverse_port: u16,
    #[structopt(env = "SAMIZDAT_DATA", long, default_value = "data/db")]
    pub data: String,
    /// Maximum number of simultaneous connections.
    #[structopt(env = "SAMIZDAT_MAX_CONNECTIONS", long, default_value = "1024")]
    pub max_connections: usize,
    /// Maximum number of _simultaneous_ resolutions per query.
    #[structopt(env = "SAMIZDAT_MAX_RESOLUTIONS_PER_QUERY", long, default_value = "12")]
    pub max_resolutions_per_query: usize,
    /// The maximum number of _simultaneous_ queries a node can make.
    #[structopt(env = "SAMIZDAT_MAX_QUERY_PER_NODE", long, default_value = "4")]
    pub max_queries_per_node: usize,
    /// The inverse of the interval that we delay if a node is requesting too many queries.
    /// (e.g., 2 => delay 500ms).
    #[structopt(env = "SAMIZDAT_MAX_QUERY_RATE_PER_NODE", long, default_value = "12")]
    pub max_query_rate_per_node: f64,
    /// The maximum number of candidates to return to the client.
    #[structopt(env = "SAMIZDAT_MAX_CANDIDATES", long, default_value = "1")]
    pub max_candidates: usize,
    /// Other servers to which to listen to.
    #[structopt(env = "SAMIZDAT_PARTNERS", long)]
    pub partners: Option<Vec<AddrToResolve>>,
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
    pub async fn resolve(&self) -> Result<(&'static str, SocketAddr), crate::Error> {
        let name = Box::leak(self.to_string().into_boxed_str());
        let addr = match self {
            AddrToResolve::SocketAddr(addr) => *addr,
            AddrToResolve::DomainAndPort(domain, port) => {
                tokio::net::lookup_host((&**domain, *port))
                    .await?
                    .next()
                    .ok_or_else(|| format!("no such host {}", domain))?
            }
        };

        Ok((name, addr))
    }
}
