mod cli;
mod commands;
mod error;
mod logger;

pub use error::Error;

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    let _ = logger::init_logger();

    cli::init_cli()?;
    cli::cli().execute().await?;

    Ok(())
}
