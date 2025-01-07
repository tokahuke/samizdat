mod acme;
mod cli;
mod html;
mod http;

use std::net::Ipv4Addr;

use cli::cli;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    cli::init_cli()?;
    samizdat_common::logger::init();

    tracing::info!("Starting SAMIZDAT proxy in folder {:?}", cli().data);

    http::validate_node_is_up().await?;

    // Run server:
    if cli().https {
        tracing::info!(
            "Proxy mode is HTTPS for domain {} in port {}",
            cli().domain()?,
            cli().port.unwrap_or(443)
        );
        samizdat_common::rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install crypto provider `ring`");
        acme::serve(
            cli().owner()?,
            cli().domain()?,
            &cli().acme_directory,
            &format!("{}/acme", cli().data),
            (Ipv4Addr::UNSPECIFIED, cli().port.unwrap_or(443)).into(),
            cli().http_port.unwrap_or(80),
            crate::http::api(),
        )
        .await?
    } else {
        tracing::info!("Proxy mode is HTTP in port {}", cli().port.unwrap_or(8080));
        axum::serve(
            tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, cli().port.unwrap_or(8080)))
                .await?,
            crate::http::api().into_make_service(),
        )
        .await?;
    }

    Ok(())
}
