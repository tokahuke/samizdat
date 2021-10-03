use std::io;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Cli {
    /// The port on which to serve the proxy. This only has effect when serving HTTP only.
    #[structopt(long)]
    pub port: Option<u16>,
    /// Whether to serve with HTTPS. This is meant for production only.
    #[structopt(long)]
    pub https: bool,
}

static mut CLI: Option<Cli> = None;

pub fn init_cli() -> Result<(), io::Error> {
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
