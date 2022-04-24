//! Command line interface for the Samizdat node.

use std::net::SocketAddr;
use std::num::ParseIntError;
use std::path::PathBuf;
use std::str::FromStr;
use structopt::StructOpt;

/// The CLI parameters.
#[derive(Debug, StructOpt)]
pub struct Cli {
    /// Path to the locally stored program data.
    #[structopt(env = "SAMIZDAT_DATA", long, default_value = "data/node")]
    pub data: PathBuf,
    /// The port on which to sever the local HTTP proxy. This is the port you will use to access in
    ///  your browser.
    #[structopt(env = "SAMIZDAT_PORT", long, default_value = "4510")]
    pub port: u16,
    /// (MB) The maximum size in bytes of the content that can be sent from a peer to this machine.
    #[structopt(env = "SAMIZDAT_MAX_CONTENT_SIZE", long, default_value = "1000")]
    pub max_content_size: usize,
    /// A list of hubs to which to connect.
    #[structopt(env = "SAMIZDAT_HUBS", long, default_value = "[::1]:4511")]
    pub hubs: Vec<AddrToResolve>,
    /// The mode of resolution to be used with domain names. Must be one of `ensure-ipv4`,
    /// `ensure-ipv6`, `prefer-ipv6`, `prefer-ipv4` or `use-both`. Note that the `prefer-*` options
    /// will resolve to the other IP version if no address is available for the current version.
    #[structopt(env = "SAMIZDAT_RESOLUTION_MODE", long, default_value = "use-both")]
    pub resolution_mode: AddrResolutionMode,
    /// The maximum number of hubs to be queried simultaneously per query.
    #[structopt(env = "SAMIZDAT_MAX_PARALLEL_HUBS", long, default_value = "3")]
    pub max_parallel_hubs: usize,
    /// (MB) The maximum total size of all cached files and _disposable_ files. Note that the total
    /// size may still exceed this value, since some of the allocated space is used to store
    /// data that is valuable to you.
    #[structopt(env = "SAMIZDAT_MAX_STORAGE", long, default_value = "1000")]
    pub max_storage: usize,
    /// The number of riddles to be sent on each query. This gives the maximum number of hops that a
    /// query can propagate inside a network, with 2 being the absolute minimum to get a result.
    #[structopt(env = "SAMIZDAT_RIDDLES_PER_QUERY", long, default_value = "6")]
    pub riddles_per_query: usize,
}

/// The handle to the CLI parameters.
static mut CLI: Option<Cli> = None;

/// Initializes the [`CLI`] with the values from the command line.
pub fn init_cli() -> Result<(), crate::Error> {
    let cli = Cli::from_args();

    log::info!("Arguments from command line: {:#?}", cli);

    std::fs::create_dir_all(&cli.data)?;

    log::debug!("Initialized data folder");

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

/// Returns a handle to the CLI arguments. Only use this after initialization.
pub fn cli<'a>() -> &'a Cli {
    unsafe { CLI.as_ref().expect("cli not initialized") }
}

#[derive(Debug, Clone, Copy)]
pub enum AddrResolutionMode {
    EnsureIpv6,
    EnsureIpv4,
    PreferIpv6,
    PreferIpv4,
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
    fn filter_hosts(self, hosts: &[SocketAddr]) -> Vec<SocketAddr> {
        // Iterator factory (makes IPs canonical).
        let iter_hosts = || {
            hosts
                .iter()
                .map(|addr| (addr.ip().to_canonical(), addr.port()))
                .map(SocketAddr::from)
        };

        match self {
            Self::EnsureIpv6 => iter_hosts().filter(SocketAddr::is_ipv6).take(1).collect(),
            Self::EnsureIpv4 => iter_hosts().filter(SocketAddr::is_ipv4).take(1).collect(),
            Self::PreferIpv6 => iter_hosts()
                .max_by_key(|addr| if addr.is_ipv6() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::PreferIpv4 => iter_hosts()
                .max_by_key(|addr| if addr.is_ipv4() { 1 } else { 0 })
                .into_iter()
                .collect(),
            Self::UseBoth => {
                let an_ipv6 = iter_hosts().filter(SocketAddr::is_ipv6).take(1);
                // Loopbacks are coerced to IPv6.
                let an_ipv4 = iter_hosts()
                    .filter(SocketAddr::is_ipv4)
                    .filter(|socket| !socket.ip().is_loopback())
                    .take(1);

                an_ipv4.chain(an_ipv6).collect()
            }
        }
    }
}

/// A flexible representation of an address in the internet.
#[derive(Debug)]
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

impl AddrToResolve {
    fn name(&self) -> String {
        match self {
            AddrToResolve::SocketAddr(addr) => addr.to_string(),
            AddrToResolve::DomainAndPort(domain, port) if *port == 4511 => domain.to_owned(),
            AddrToResolve::DomainAndPort(domain, port) => format!("{}:{}", domain, port),
        }
    }

    pub async fn resolve(
        &self,
        resolution_mode: AddrResolutionMode,
    ) -> Result<impl Iterator<Item = (&'static str, SocketAddr)>, crate::Error> {
        let name: &'static str = Box::leak(self.name().into_boxed_str());

        let addrs = match self {
            AddrToResolve::SocketAddr(addr) => vec![*addr],
            AddrToResolve::DomainAndPort(domain, port) => {
                let hosts = resolution_mode.filter_hosts(
                    &tokio::net::lookup_host((&**domain, *port))
                        .await?
                        .collect::<Vec<_>>(),
                );

                if hosts.is_empty() {
                    return Err(format!("no such host {}", domain).into());
                } else {
                    hosts
                }
            }
        };

        Ok(addrs.into_iter().map(move |addr| (name, addr)))
    }
}
