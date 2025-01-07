use anyhow::Context;
use axum::extract::Path;
use axum::response::Redirect;
use axum::routing::any;
use axum::Router;
use rustls_acme::caches::DirCache;
use rustls_acme::AcmeConfig;
use std::net::SocketAddr;
use std::sync::OnceLock;
use tokio::net::TcpListener;
use tokio_stream::StreamExt;

/// Serves an axum app using TLS:
pub async fn serve(
    owner: &str,
    domain: &str,
    directory: &str,
    acme_cache: &str,
    addr: SocketAddr,
    http_port: u16,
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

    let mut http_addrs = addr.clone();
    http_addrs.set_port(http_port);
    let http_listener = TcpListener::bind(http_addrs).await?;

    let (https_outcome, http_outcome) = tokio::join!(
        axum_server::bind(addr)
            .acceptor(acceptor)
            .serve(app.into_make_service()),
        axum::serve(
            http_listener,
            redirect_to_https(domain, addr).into_make_service(),
        )
    );

    http_outcome.context("serving the HTTP server")?;
    https_outcome.context("serving the HTTPS server")?;

    Ok(())
}

fn redirect_to_https(domain: &str, addr: SocketAddr) -> axum::Router {
    static BASE_URL: OnceLock<String> = OnceLock::new();

    BASE_URL
        .set(if addr.port() == 443 {
            format!("https://{domain}/")
        } else {
            format!("https://{domain}:{}/", addr.port())
        })
        .expect("can only call `redirect_to_https` once!");

    axum::Router::new()
        .route(
            "/{*path}",
            any(|Path(path): Path<String>| async move {
                Redirect::permanent(&format!(
                    "{}{path}",
                    BASE_URL.get().expect("base url was set")
                ))
            }),
        )
        .route(
            "/",
            any(|| async move {
                Redirect::permanent(BASE_URL.get().as_ref().expect("base url was set"))
            }),
        )
}
