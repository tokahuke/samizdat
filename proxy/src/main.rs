mod acme;
mod cli;
mod html;
mod http;

use std::net::Ipv4Addr;

use cli::cli;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();
    cli::init_cli()?;

    http::validate_node_is_up().await?;

    // Run server:
    if cli().https {
        samizdat_common::rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install crypto provider `ring`");
        acme::serve(
            cli().owner()?,
            cli().domain()?,
            &cli().acme_directory,
            &format!("{}/acme", cli().data),
            (Ipv4Addr::UNSPECIFIED, cli().port.unwrap_or(443)).into(),
            crate::http::api(),
        )
        .await?
    } else {
        // Start server:
        axum::serve(
            tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, cli().port.unwrap_or(8080)))
                .await?,
            crate::http::api().into_make_service(),
        )
        .await?;
    }

    Ok(())
}
