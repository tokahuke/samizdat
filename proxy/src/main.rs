mod acme;
mod cli;
mod dns;
mod html;
mod http;
mod wildcard;

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
        samizdat_common::rustls::crypto::ring::default_provider()
            .install_default()
            .expect("failed to install crypto provider `ring`");

        if let Some(dns) = cli().dns.as_ref() {
            tracing::info!(
                "Proxy mode is HTTPS (wildcard) for *.{} in port {}",
                dns.wildcard_root,
                cli().port.unwrap_or(443)
            );
            wildcard::serve(
                dns,
                cli().owner()?,
                &cli().acme_directory,
                std::path::PathBuf::from(format!("{}/wildcard", cli().data)),
                (Ipv4Addr::UNSPECIFIED, cli().port.unwrap_or(443)).into(),
                cli().http_port.unwrap_or(80),
                crate::http::wildcard_api(dns.wildcard_root.clone()),
            )
            .await?
        } else {
            tracing::info!(
                "Proxy mode is HTTPS for domain {} in port {}",
                cli().domain()?,
                cli().port.unwrap_or(443)
            );
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
        }
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
