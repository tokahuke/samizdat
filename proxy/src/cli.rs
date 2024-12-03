use std::sync::LazyLock;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Cli {
    /// The port on which to serve the proxy. This only has effect when serving HTTP only.
    #[structopt(long)]
    pub port: Option<u16>,
    /// Whether to serve with HTTPS. This is meant for production only.
    #[structopt(long)]
    pub https: bool,
    // /// An alternative port to run an HTTP server that redirects to HTTPS (only applicable
    // /// if HTTPS is set).
    // #[structopt(long)]
    // pub http_port: Option<u16>,
    /// The name of the domain that this proxy will serve (only applicable if HTTPS is
    /// set).
    #[structopt(long)]
    pub domain: Option<String>,
    /// The e-mail of the owned of the domain (this will be passed to `certbot`; only
    /// applicable if HTTPS is set).
    #[structopt(long)]
    pub owner: Option<String>,
}

impl Cli {
    pub fn domain(&self) -> Result<&str, anyhow::Error> {
        let Some(domain) = self.domain.as_ref() else {
            anyhow::bail!("missing domain parameter")
        };

        Ok(&domain)
    }

    pub fn owner(&self) -> Result<&str, anyhow::Error> {
        let Some(owner) = self.owner.as_ref() else {
            anyhow::bail!("missing owner parameter")
        };

        Ok(&owner)
    }
}

static CLI: LazyLock<Cli> = LazyLock::new(|| {
    let cli = Cli::from_args();
    tracing::info!("Arguments from command line: {:#?}", cli);
    cli
});

pub fn cli<'a>() -> &'a Cli {
    &CLI
}
