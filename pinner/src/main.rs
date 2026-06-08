mod cli;
mod db;
mod expiry;
mod http;
mod node_client;

use std::net::Ipv4Addr;

use cli::cli;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    cli::init_cli()?;
    samizdat_common::logger::init();

    tracing::info!("Starting SAMIZDAT pinner in folder {:?}", cli().data);

    db::init(&cli().data)?;
    node_client::init(&cli().node)?;

    tokio::spawn(expiry::run());

    let app = http::router();
    let addr = (Ipv4Addr::UNSPECIFIED, cli().port);
    tracing::info!("Pinner listening on http://{}:{}", addr.0, addr.1);

    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app.into_make_service(),
    )
    .await?;

    Ok(())
}
