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
                None => {
                    // The acme state stream returning `None` means cert renewal
                    // has stopped. The server stays up serving with the
                    // (eventually expired) certificate; without this loud log
                    // the failure is silent until clients start getting
                    // CERT_HAS_EXPIRED errors.
                    tracing::error!(
                        "ACME state stream ended; certificate renewal has STOPPED. \
                         The proxy will keep serving with the current cert until it \
                         expires. Restart the proxy to restore renewal."
                    );
                    break;
                }
            }
        }
    });

    let mut http_addrs = addr;
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

/// Build a port-80 -> 443 redirector that does not allocate a per-process
/// `OnceLock`. The wildcard cert path reuses this helper because its
/// rustls handoff happens in a separate code path from the rustls-acme
/// HTTP-01 path.
pub fn redirect_to_https_for(domain: &str, addr: SocketAddr) -> axum::Router {
    let base = if addr.port() == 443 {
        format!("https://{domain}/")
    } else {
        format!("https://{domain}:{}/", addr.port())
    };
    let base_for_path = base.clone();
    let base_for_root = base;
    axum::Router::new()
        .route(
            "/{*path}",
            any(move |Path(path): Path<String>| {
                let base = base_for_path.clone();
                async move { Redirect::permanent(&format!("{base}{path}")) }
            }),
        )
        .route(
            "/",
            any(move || {
                let base = base_for_root.clone();
                async move { Redirect::permanent(&base) }
            }),
        )
}
