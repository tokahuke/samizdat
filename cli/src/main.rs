mod cli;
mod commands;
mod error;
mod logger;
mod util;
mod manifest;

pub use error::Error;
pub use manifest::Manifest;

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    let _ = logger::init_logger();

    cli::init_cli()?;
    cli::cli().execute().await?;

    Ok(())
}
