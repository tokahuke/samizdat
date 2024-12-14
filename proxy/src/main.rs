mod cli;
mod html;
mod http;

use std::time::Duration;

use axum_server::{tls_rustls::RustlsConfig, Handle};
use cli::cli;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();
    cli::init_cli()?;

    // Run server:
    if cli().https {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install crypto provider `aws_lc_rs`");

        loop {
            // Run certbot:
            let status = std::process::Command::new("certbot")
                .arg("certonly")
                .arg("--standalone")
                .arg("--non-interactive")
                .arg("--email")
                .arg(cli().owner()?)
                .arg("--agree-tos")
                .arg("--domain")
                .arg(cli().domain()?)
                .spawn()
                .expect("failed to spawn certbot")
                .wait()
                .expect("failed to run certbot");
            assert_eq!(status.code().expect("there always is a code (?)"), 0);

            let config = RustlsConfig::from_pem_file(
                format!("/etc/letsencrypt/live/{}/fullchain.pem", cli().domain()?),
                format!("/etc/letsencrypt/live/{}/privkey.pem", cli().domain()?),
            )
            .await?;
            let handle = Handle::new();
            handle.graceful_shutdown(Some(Duration::from_secs(60 * 24 * 60 * 60)));

            axum_server::bind_rustls(([0, 0, 0, 0], cli().port.unwrap_or(443)).into(), config)
                .handle(handle)
                .serve(crate::http::api().into_make_service())
                .await?;
        }
    } else {
        // Start server:
        axum::serve(
            tokio::net::TcpListener::bind(("0.0.0.0", cli().port.unwrap_or(8080))).await?,
            crate::http::api().into_make_service(),
        )
        .await?;
    }

    Ok(())
}
