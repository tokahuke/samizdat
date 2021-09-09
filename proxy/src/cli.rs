use std::io;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Cli {
    /// The port on which to serve the proxy.
    #[structopt(long, default_value = "8080")]
    pub port: u16,
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
