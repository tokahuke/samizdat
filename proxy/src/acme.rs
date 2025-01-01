use axum::Router;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig;
use std::net::SocketAddr;
use tokio_stream::StreamExt;

/// Serves an axum app using TLS:
pub async fn serve(
    owner: &str,
    domain: &str,
    directory: &str,
    acme_cache: &str,
    addr: SocketAddr,
    app: Router,
) -> Result<(), anyhow::Error> {
    let mut state = AcmeConfig::new([domain])
        .contact([format!("mailto:{owner}")])
        .cache_option(Some(DirCache::new(acme_cache.to_owned())))
        .directory(directory)
        .state();
    let acceptor = state.axum_acceptor(state.default_rustls_config());

    tokio::spawn(async move {
        loop {
            match state.next().await {
                Some(Ok(ok)) => tracing::info!("acme event: {:?}", ok),
                Some(Err(err)) => tracing::error!("acme error: {:?}", err),
                None => break,
            }
        }
    });

    axum_server::bind(addr)
        .acceptor(acceptor)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}
