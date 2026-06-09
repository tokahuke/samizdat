//! HTTP API for the Samizdat Node.

mod auth;
mod collections;
mod connections;
mod content;
mod editions;
mod ethereum_provider;
mod host_scope;
mod hubs;
mod objects;
mod peers;
mod redirects;
mod resolvers;
mod series;
mod series_owners;
mod subscriptions;

use std::{
    convert::Infallible,
    net::{Ipv6Addr, SocketAddr},
    num::ParseIntError,
    time::Duration,
};

use axum::{
    Router,
    extract::{ConnectInfo, FromRequestParts, Request},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures::FutureExt;
use http::request::Parts;
use redirects::redirect_request;

use crate::cli;

/// Gets the corresponding HTTP status code for a Samizdat error.
fn error_status_code(err: &crate::Error) -> http::StatusCode {
    match err {
        crate::Error::Message(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Rpc(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::Base64(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Io(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::BadHashLength(_) => http::StatusCode::BAD_REQUEST,
        crate::Error::Bincode(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::QuicConnectionError(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        crate::Error::AllCandidatesFailed => http::StatusCode::BAD_GATEWAY,
        crate::Error::InvalidCollectionItem => http::StatusCode::BAD_REQUEST,
        crate::Error::InvalidEdition => http::StatusCode::BAD_REQUEST,
        crate::Error::DifferentPublicKeys => http::StatusCode::BAD_REQUEST,
        crate::Error::NoHeaderRead => http::StatusCode::INTERNAL_SERVER_ERROR,
        _ => http::StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// 400-rejection carrying the parse error for a malformed
/// X-Samizdat-Timeout header.
struct SamizdatTimeoutRejection(ParseIntError);

impl IntoResponse for SamizdatTimeoutRejection {
    fn into_response(self) -> Response {
        Response::builder()
            .status(400)
            .body(format!("Bad X-Samizdat-Timout header value: {}", self.0).into())
            .expect("can build error response")
    }
}

/// Parsed timeout from the X-Samizdat-Timeout header, defaulting to
/// 10 seconds when the header is missing.
struct SamizdatTimeout(Duration);

impl<S: Send + Sync> FromRequestParts<S> for SamizdatTimeout {
    type Rejection = SamizdatTimeoutRejection;
    async fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> Result<SamizdatTimeout, Self::Rejection> {
        parts
            .headers
            .get("X-Samizdat-Timeout")
            .map(|header| {
                String::from_utf8_lossy(header.as_bytes())
                    .parse::<u64>()
                    .map(Duration::from_secs)
            })
            .unwrap_or(Ok(Duration::from_secs(10)))
            .map(SamizdatTimeout)
            .map_err(SamizdatTimeoutRejection)
    }
}

/// The request's Content-Type, defaulting to `application/octet-stream`
/// when the header is missing.
struct ContentType(String);

impl<S: Send + Sync> FromRequestParts<S> for ContentType {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<ContentType, Self::Rejection> {
        Ok(parts
            .headers
            .get("Content-Type")
            .map(|header| String::from_utf8_lossy(header.as_bytes()).into_owned())
            .map(ContentType)
            .unwrap_or_else(|| ContentType("application/octet-stream".to_owned())))
    }
}

/// The standardized JSON reply for the API.
pub struct ApiResponse<T>(Result<T, crate::Error>);

impl<T> IntoResponse for ApiResponse<T>
where
    T: serde::Serialize,
{
    fn into_response(self) -> Response {
        let status = self
            .0
            .as_ref()
            .map_err(error_status_code)
            .err()
            .unwrap_or_default();
        let json = self.0.map_err(|err| err.to_string());

        Response::builder()
            .status(status)
            .body(
                serde_json::to_string(&json)
                    .expect("can serialize API response")
                    .into(),
            )
            .expect("can create API response")
    }
}

/// A response that is not a response from the API directly, but "anything else". Used
/// mainly for serving content.
pub struct PageResponse(Result<Response, crate::Error>);

impl IntoResponse for PageResponse {
    fn into_response(self) -> Response {
        match self.0 {
            Ok(response) => response,
            Err(err) => Response::builder()
                .status(error_status_code(&err))
                .body(err.to_string().into())
                .expect("can build error response"),
        }
    }
}

/// The entrypoint of the Samizdat node public HTTP API.
///
/// Split into two sub-routers and a top-level CORS layer:
///
/// - The **admin sub-router** holds every `/_*` nest. Its requests must arrive on the
///   bare loopback host (`localhost`, `127.0.0.1`, `[::1]`); the `require_bare_host`
///   layer returns 404 for admin requests on any `*.localhost` subdomain. Admin auth
///   (bearer token or `/_register` trusted-context grant via the Referer) still applies
///   on individual routes; this layer is only the host-level guard.
/// - The **content sub-router** holds the two content handlers (`GET /` and `GET
///   /{*name}`). Both use the `HostScope` extractor to dispatch: bare host serves the
///   welcome HTML at `/`, subdomain hosts resolve series / identity content. The content
///   router never sees an admin path because the admin nest above is checked first by
///   `Router::merge` order.
/// - The top-level CORS layer reflects any `Origin` whose host is `localhost` or ends in
///   `.localhost`, so the JS SDK can keep talking from content subdomains to admin
///   endpoints. Tightening is a followup (see `docs/browser-security.md`).
fn api() -> Router {
    use http::HeaderValue;
    use tower::ServiceBuilder;
    use tower_http::set_header::SetResponseHeaderLayer;

    // Admin-only protections.
    //
    // * `X-Frame-Options: DENY` refuses to be framed; closes the clickjacking surface against
    //   the consent grant flow.
    // * `Content-Security-Policy` locks the admin origin to same-origin resources only. The
    //   admin host serves samizdat's own UI (welcome page, `/_register`, `/_doctor`, JSON
    //   APIs) -- no author-uploaded content runs here -- so a strict policy costs nothing.
    //   `'unsafe-inline'` is allowed for the inline scripts and styles inside samizdat's own
    //   templates; tightening that to hash- or nonce-based is a followup if those templates
    //   stop being touched.
    let admin_layers = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            http::header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            http::header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'none'; \
                 script-src 'self' 'unsafe-inline'; \
                 style-src 'self' 'unsafe-inline'; \
                 connect-src 'self'; \
                 img-src 'self' data:; \
                 form-action 'self'; \
                 frame-ancestors 'none'; \
                 base-uri 'none'",
            ),
        ))
        .layer(axum::middleware::from_fn(require_bare_host));

    let admin = Router::new()
        .nest("/_objects", objects::api())
        .nest("/_collections", collections::api())
        .nest("/_series", series::api())
        .nest("/_series-owners", series_owners::api())
        .nest("/_editions", editions::api())
        .nest("/_subscriptions", subscriptions::api())
        .nest("/_ethereum-provider", ethereum_provider::api())
        .nest("/_auth", auth::api())
        .nest("/_hubs", hubs::api())
        .nest("/_connections", connections::api())
        .nest("/_peers", peers::api())
        .nest("/_vacuum", vacuum())
        .layer(admin_layers);

    let content = Router::new()
        .route("/", get(content::content_root))
        .route("/{*name}", get(content::content_path));

    // Global protections applied to admin + content.
    //
    // * `X-Content-Type-Options: nosniff` blocks MIME sniffing so a series that uploads a
    //   file with a forged Content-Type cannot trick the browser into executing it as HTML,
    //   JS, or SVG-with-script.
    // * `Referrer-Policy: same-origin` strips Referer on cross-origin requests, keeping it
    //   intact same-origin so the `/_register` trusted-context check still works. Authors
    //   override per-document via `<meta name="referrer">` or per-element via
    //   `referrerpolicy`.
    // * `Permissions-Policy: interest-cohort=()` opts content out of Chrome's Topics / FLoC
    //   behavioral cohort.
    // * `cors_layer()` reflects any `Origin` whose host is `localhost` or `*.localhost`, so
    //   the JS SDK can keep talking from content subdomains to admin endpoints; tightening is
    //   a followup (see `docs/browser-security.md`).
    let global_layers = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            http::header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            http::header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            http::header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("interest-cohort=()"),
        ))
        .layer(cors_layer());

    admin.merge(content).layer(global_layers)
}

/// Builds the CORS layer applied at the top of the node API. Reflects any
/// `Origin` whose host is `localhost` or a `*.localhost` subdomain; rejects
/// everything else at the browser. Credentials enabled so the SDK can carry
/// the bearer-cookie path (deferred). Permissive on purpose: per-route
/// scoping is part of the SDK rework followup tracked in
/// `docs/browser-security.md`.
fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{AllowOrigin, CorsLayer};
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            let Ok(s) = origin.to_str() else {
                return false;
            };
            let Ok(url) = url::Url::parse(s) else {
                return false;
            };
            match url.host() {
                Some(url::Host::Domain(d)) => d == "localhost" || d.ends_with(".localhost"),
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.to_canonical().is_loopback(),
                None => false,
            }
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

/// Layer applied to the admin sub-router that returns 404 for any request
/// arriving on a `*.localhost` subdomain. Admin endpoints exist only on the
/// bare loopback host; a subdomain request that happens to hit an admin
/// path should not be served, otherwise it would defeat the per-series
/// origin isolation (a content page could call admin endpoints from its own
/// origin via a same-origin fetch).
async fn require_bare_host(request: Request, next: Next) -> Response {
    use crate::http::host_scope::{HostScope, classify};
    let host = request
        .headers()
        .get("host")
        .and_then(|h| std::str::from_utf8(h.as_bytes()).ok());
    let on_bare = match host {
        Some(raw) => matches!(classify(raw), Ok(HostScope::BareLoopback)),
        None => false,
    };
    if on_bare {
        next.run(request).await
    } else {
        Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .body(
                "admin endpoints are only available on the bare loopback host \
                 (`localhost`, `127.0.0.1`, `[::1]`)"
                    .into(),
            )
            .expect("can build require_bare_host response")
    }
}

/// Router for the vacuum endpoints: trigger a manual vacuum or flush the
/// entire object store.
///
/// Gated by `authenticate_trusted_context`: either the request comes from the
/// `/_register` trusted page OR it carries a valid bearer token (the local CLI does the
/// latter). Without this gate a same-origin malicious page could trigger
/// `/_vacuum/flush-all` via a simple cross-origin POST (which bypasses CORS preflight
/// when the Content-Type is `text/plain`) and erase the entire object store.
fn vacuum() -> Router {
    Router::new()
        .route(
            "/",
            post(|| async { crate::vacuum::vacuum() }.map(ApiResponse)),
        )
        .route(
            "/flush-all",
            post(|| {
                async {
                    crate::vacuum::flush_all();
                    Ok(())
                }
                .map(ApiResponse)
            }),
        )
        .layer(axum::middleware::from_fn(
            auth::authenticate_trusted_context,
        ))
}

/// Middleware function to restrict access to only local connections.
///
/// # Arguments
/// * `addr` - Socket address information of the incoming connection
/// * `request` - The incoming HTTP request
/// * `next` - The next middleware in the chain
///
/// # Returns
/// Returns a 403 Forbidden response for non-loopback addresses, otherwise
/// continues the middleware chain.
async fn deny_outside_requests(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if !addr.ip().to_canonical().is_loopback() {
        return Response::builder()
            .status(http::StatusCode::FORBIDDEN)
            .body("403 Forbidden".into())
            .expect("can build stadard error message");
    }

    next.run(request).await
}

/// Runs the HTTP API server.
pub async fn serve() -> Result<(), crate::Error> {
    let server = api().layer(
        tower::ServiceBuilder::new()
            .layer(axum::middleware::from_fn(deny_outside_requests))
            .layer(axum::middleware::from_fn(redirect_request))
            .layer(tower_http::trace::TraceLayer::new_for_http()),
    );

    axum::serve(
        tokio::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, cli().port)).await?,
        server.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
