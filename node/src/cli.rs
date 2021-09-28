use std::net::SocketAddr;
use std::num::ParseIntError;
use std::str::FromStr;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Cli {
    /// Path to the locally stored program data.
    #[structopt(env, long, default_value = "data/db")]
    pub db_path: String,
    /// The port on which to sever the local HTTP proxy. This is the port you will use to access in
    ///  your browser.
    #[structopt(env, long, default_value = "4510")]
    pub port: u16,
    /// The maximum size in bytes of the content that can be sent from a peer to this machine.
    #[structopt(env, long, default_value = "1000000000")]
    pub max_content_size: usize,
    /// A list of hubs to which to connect.
    #[structopt(env, long, default_value = "[::1]:4511")]
    pub hubs: Vec<AddrToResolve>,
    /// The maximum number of hubs to be queried simultaneously per query.
    #[structopt(env, long, default_value = "3")]
    pub max_parallel_hubs: usize,
    /// The maximum total size of all cached files and _disposable_ files. Note that the total
    ///  size may still exceed this value, since some of the allocated space is used to store
    /// data that is valuable to you.
    #[structopt(env, long, default_value = "1000000000")]
    pub max_storage: usize,
}

static mut CLI: Option<Cli> = None;

pub fn init_cli() -> Result<(), crate::Error> {
    let cli = Cli::from_args();

    log::info!("Arguments from command line: {:#?}", cli);

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

pub fn cli<'a>() -> &'a Cli {
    unsafe { CLI.as_ref().expect("cli not initialized") }
}

#[derive(Debug)]
pub enum AddrToResolve {
    SocketAddr(SocketAddr),
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

    pub async fn resolve(&self) -> Result<(&'static str, SocketAddr), crate::Error> {
        let name = Box::leak(self.name().into_boxed_str());
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
