//! Authentication API for web applications.

use askama::Template;
use axum::extract::{FromRequestParts, Path, Request};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, patch};
use axum::{Json, Router};
use axum_extra::extract::Query as AxumExtraQuery;
use futures::FutureExt;
use http::request::Parts;
use samizdat_common::db::{readonly_tx, writable_tx, Table as _};
use serde_derive::{Deserialize, Serialize};
use url::{Host, Origin, Url};

use crate::access::{access_token, AccessRight, Entity};
use crate::db::Table;

use super::ApiResponse;

/// The authentication management API.
pub fn api() -> Router {
    Router::new()
        .merge(get_auth_current())
        .merge(get_auths())
        .merge(get_auth())
        .merge(patch_auth())
        .merge(delete_auth())
        .merge(get_register())
}

/// Gets the access rights for a given entity. This can only be called from a trusted
/// context.
fn get_auth() -> Router {
    Router::new().route(
        "/{*tail}",
        get(|Path(tail): Path<String>| {
            async move {
                let entity = Entity::from_path(tail.as_str()).ok_or("not an entity")?;
                let serialized = bincode::serialize(&entity).expect("can serialize");
                let current: Vec<AccessRight> = readonly_tx(|tx| {
                    Table::AccessRights.get(tx, serialized, |rights| bincode::deserialize(rights))
                })
                .transpose()?
                .unwrap_or_default();

                Ok(current)
            }
            .map(ApiResponse)
        })
        .layer(middleware::from_fn(authenticate_trusted_context)),
    )
}

/// Gets the access rights for the current entity (current entity is decided based on
/// the `Referer` header).
fn get_auth_current() -> Router {
    Router::new().route(
        "/_current",
        get(|SecurityScope(entity): SecurityScope| {
            async move {
                let serialized = bincode::serialize(&entity).expect("can serialize");
                let current: Vec<AccessRight> = readonly_tx(|tx| {
                    Table::AccessRights.get(tx, serialized, |rights| bincode::deserialize(rights))
                })
                .transpose()?
                .unwrap_or_default();

                Ok(current)
            }
            .map(ApiResponse)
        })
        .layer(crate::security_scope!(AccessRight::Public)),
    )
}

/// Gets the list of all entities and all associated access rights.
fn get_auths() -> Router {
    #[derive(Serialize)]
    struct Response {
        entity: Entity,
        granted_rights: Vec<AccessRight>,
    }

    Router::new().route(
        "/",
        get(|| {
            async move {
                let all_auths = readonly_tx(|tx| {
                    Table::AccessRights
                        .range(..)
                        .collect::<_, Result<Vec<_>, crate::Error>, _, _>(tx, |key, value| {
                            let entity: Entity = bincode::deserialize(key)?;
                            let granted_rights: Vec<AccessRight> = bincode::deserialize(value)?;
                            Ok(Response {
                                entity,
                                granted_rights,
                            })
                        })
                })?;

                Ok(all_auths)
            }
            .map(ApiResponse)
        })
        .layer(middleware::from_fn(authenticate_trusted_context)),
    )
}

/// Changes (or sets) the access rights for a given entity. This can only be called from
/// a trusted context.
fn patch_auth() -> Router {
    #[derive(Debug, Deserialize)]
    struct Request {
        granted_rights: Vec<AccessRight>,
    }

    Router::new().route(
        "/{*tail}",
        patch(|Path(tail): Path<String>, Json(request): Json<Request>| {
            async move {
                let entity = Entity::from_path(tail.as_str()).ok_or("not an entity")?;
                let serialized = bincode::serialize(&entity).expect("can serialize");
                let mut current: Vec<AccessRight> = readonly_tx(|tx| {
                    Table::AccessRights.get(tx, &serialized, |rights| bincode::deserialize(rights))
                })
                .transpose()?
                .unwrap_or_default();

                current.extend(request.granted_rights);
                current.sort_unstable_by_key(|right| *right as u8);
                current.dedup();

                writable_tx(|tx| {
                    Table::AccessRights.put(
                        tx,
                        serialized,
                        bincode::serialize(&current).expect("can serialize"),
                    );

                    Ok(true)
                })
            }
            .map(ApiResponse)
        })
        .layer(middleware::from_fn(authenticate_trusted_context)),
    )
}

/// Revokes all access rights for a given entity.
fn delete_auth() -> Router {
    Router::new().route(
        "/{*tail}",
        delete(|Path(tail): Path<String>| {
            async move {
                let entity = Entity::from_path(tail.as_str()).ok_or("not an entity")?;

                writable_tx(|tx| {
                    Table::AccessRights
                        .delete(tx, bincode::serialize(&entity).expect("can serialize"));
                    Ok(())
                })
                .expect("infalible");

                Ok(true) as Result<_, crate::Error>
            }
            .map(ApiResponse)
        })
        .layer(crate::security_scope!()),
    )
}

/// Checks whether the request is _really_ coming from Samizdat. This is a complement
/// to CORS.
fn check_origin(referrer: &Url) -> Result<(), Origin> {
    let origin = referrer.origin();

    // Find out if some cross-origin thing is trying ot trick you.
    match &origin {
        url::Origin::Tuple(http, host, _) if http == "http" || http == "https" => match host {
            Host::Domain(domain) if domain == "localhost" => return Ok(()),
            Host::Ipv4(ip) if ip.is_loopback() => return Ok(()),
            Host::Ipv6(ip) if ip.to_canonical().is_loopback() => return Ok(()),
            _ => {}
        },
        _ => {}
    }

    Err(origin)
}

/// Paths which are *always* trusted.
fn is_trusted_context(referrer: &Url) -> bool {
    ["/_register"].contains(&referrer.path())
}

/// Returns `Ok(None)` when trusted context.
fn entity_from_referrer(referrer: &Url) -> Result<Entity, SecurityScopeRejection> {
    check_origin(referrer).map_err(SecurityScopeRejection::BadOrigin)?;

    if is_trusted_context(referrer) {
        return Err(SecurityScopeRejection::TrustedContext(referrer.to_owned()));
    }

    let Some(entity) = Entity::from_path(referrer.path()) else {
        return Err(SecurityScopeRejection::NotAnEntity(
            referrer.path().to_owned(),
        ));
    };

    Ok(entity)
}

/// Extracts the Referer URL from a request.
///
/// # Arguments
/// * `request` - The HTTP request to extract from
///
/// # Returns
/// Ok(Some(Url)) if Referer exists and is valid, Ok(None) if no Referer header present,
/// or Err(SecurityScopeRejection::UrlParseError) if the Referer header value is invalid
fn referer_from_request(request: &Request) -> Result<Option<Url>, SecurityScopeRejection> {
    let Some(header) = request.headers().get("referer") else {
        return Ok(None);
    };
    String::from_utf8_lossy(header.as_bytes())
        .parse::<Url>()
        .map(Some)
        .map_err(SecurityScopeRejection::UrlParseError)
}
/// Extracts the Referer URL from request parts.
///
/// # Arguments
/// * `parts` - The HTTP request parts containing headers
///
/// # Returns
/// Ok(Some(Url)) if Referer exists and is valid, Ok(None) if no Referer header present,
/// or Err(SecurityScopeRejection) if the Referer header value is invalid UTF-8 or malformed URL
fn referer_from_parts(parts: &Parts) -> Result<Option<Url>, SecurityScopeRejection> {
    let Some(header) = parts.headers.get("referer") else {
        return Ok(None);
    };
    String::from_utf8_lossy(header.as_bytes())
        .parse::<Url>()
        .map(Some)
        .map_err(SecurityScopeRejection::UrlParseError)
}

/// Extracts the entity from a request's Referer header.
///
/// # Arguments
/// * `request` - The HTTP request to extract from
///
/// # Returns
/// Ok(Some(Entity)) if entity was found and validated, Ok(None) if no Referer header present,
/// or Err(SecurityScopeRejection) if the Referer is invalid, has a bad origin, or doesn't
/// correspond to an entity
fn entity_from_request(request: &Request) -> Result<Option<Entity>, SecurityScopeRejection> {
    let Some(referer) = referer_from_request(request)? else {
        return Ok(None);
    };
    entity_from_referrer(&referer).map(Some)
}

/// Represents the security scope of a request, containing the associated entity.
pub struct SecurityScope(pub Entity);

impl<S: Send + Sync> FromRequestParts<S> for SecurityScope {
    type Rejection = SecurityScopeRejection;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let Some(referer) = referer_from_parts(parts)? else {
            return Err(SecurityScopeRejection::MissingReferer);
        };
        entity_from_referrer(&referer).map(SecurityScope)
    }
}

/// Rejection types for security scope authentication failures.
pub enum SecurityScopeRejection {
    /// Referer header not sent.
    MissingReferer,
    /// The origin of the `Referer` header does not corresponds to the Samizdat origin.
    BadOrigin(Origin),
    /// The referer header contains an invalid url.
    UrlParseError(url::ParseError),
    /// The `Referer` header path does not corresponds to an entity.
    NotAnEntity(String),
    /// The call was expected to be done only from an untrusted context, but it was done
    /// from a trusted context.
    TrustedContext(Url),
    /// Not enough privilege to perform the call.
    InsufficientPrivilege,
    /// The call was expected to be done only from a trusted context, but it was done
    /// from an untrusted context.
    NotTrustedContext(Url),
}

impl IntoResponse for SecurityScopeRejection {
    fn into_response(self) -> Response {
        let response = Response::builder();

        match self {
            SecurityScopeRejection::MissingReferer => {
                response.status(401).body("missing referer header".into())
            }
            SecurityScopeRejection::BadOrigin(origin) => response
                .status(403)
                .body(format!("bad origin (not local): {origin:?}").into()),
            SecurityScopeRejection::UrlParseError(err) => response
                .status(400)
                .body(format!("referer header parse error: {err}").into()),
            SecurityScopeRejection::NotAnEntity(bad_path) => response
                .status(400)
                .body(format!("not an entity: {bad_path}").into()),
            SecurityScopeRejection::TrustedContext(url) => response.status(403).body(
                format!("call can only be done from an untrusted context, got: {url}").into(),
            ),
            SecurityScopeRejection::InsufficientPrivilege => {
                response.status(403).body("insuficient privilege".into())
            }
            SecurityScopeRejection::NotTrustedContext(url) => response
                .status(403)
                .body(format!("call cannot be done from an untrusted context, got: {url}").into()),
        }
        .expect("can create rejection response")
    }
}

/// Rejection types for authentication failures.
pub enum AuthenticationRejection {
    /// Authorization header is missing.
    MissingAuthorization,
    /// The provided authentication token is invalid.
    BadToken,
}

impl IntoResponse for AuthenticationRejection {
    fn into_response(self) -> Response {
        let response = Response::builder();

        match self {
            AuthenticationRejection::MissingAuthorization => response
                .status(401)
                .body("missing authorization header".into())
                .expect("can create error response"),
            AuthenticationRejection::BadToken => response
                .status(403)
                .body("bad auth token".into())
                .expect("can create error response"),
        }
    }
}

/// Combines multiple authentication rejection types into a single response.
///
/// This function handles cases where both security scope and authorization checks fail,
/// prioritizing certain rejection types over others to provide the most relevant error
/// response to the client.
///
/// # Arguments
/// * `security_scope_rejection` - The security scope rejection
/// * `authorization_rejection` - The authorization rejection
///
/// # Returns
/// A Response with appropriate status code and error message based on the rejection types:
/// - Returns the security scope rejection if authorization is just missing
/// - Returns the authorization rejection if security scope is just missing
/// - Returns a 400 Bad Request if both checks actively failed
fn merge_rejections(
    security_scope_rejection: SecurityScopeRejection,
    authorization_rejection: AuthenticationRejection,
) -> Response {
    if matches!(
        authorization_rejection,
        AuthenticationRejection::MissingAuthorization
    ) {
        security_scope_rejection.into_response()
    } else if matches!(
        security_scope_rejection,
        SecurityScopeRejection::MissingReferer
    ) {
        authorization_rejection.into_response()
    } else {
        Response::builder()
            .status(400)
            .body("mulitple authorization methods supplied and all failed".into())
            .expect("can build error response")
    }
}

/// Authenticates a call using bearer authorization using the access token. This is
/// intended for use of local applications (e.g, the Samizdat CLI).
fn do_authenticate_authorization(request: &Request) -> Result<(), AuthenticationRejection> {
    let authorization = String::from_utf8_lossy(
        request
            .headers()
            .get("authorization")
            .ok_or(AuthenticationRejection::MissingAuthorization)?
            .as_bytes(),
    );

    let token = authorization
        .trim_start_matches("Bearer ")
        .trim_start_matches("bearer ");

    if token == access_token() {
        Ok(())
    } else {
        Err(AuthenticationRejection::BadToken)
    }
}

/// Authenticates a request using both security scope and authorization methods.
///
/// # Arguments
/// * `required_rights` - Array of access rights required for the operation
/// * `request` - The incoming HTTP request
/// * `next` - The next middleware in the chain
///
/// # Returns
/// Returns a rejection response if authentication fails, otherwise continues the
/// middleware chain.
pub async fn authenticate_security_scope<const N: usize>(
    required_rights: [AccessRight; N],
    request: Request,
    next: Next,
) -> Response {
    let security_scope = do_authenticate_security_scope(required_rights, &request);
    let authorization = do_authenticate_authorization(&request);

    if let Some((security_scope_rejection, authorization_rejection)) =
        security_scope.err().zip(authorization.err())
    {
        return merge_rejections(security_scope_rejection, authorization_rejection);
    }

    next.run(request).await
}

#[macro_export]
macro_rules! security_scope {
    ($($right:expr),*) => {
        axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| {
                $crate::http::auth::authenticate_security_scope([$($right,)*], request, next)
            }
        )
    };
}

/// Performs security scope authentication for a request.
///
/// # Arguments
/// * `required_rights` - Array of access rights required for the operation
/// * `request` - The incoming HTTP request
///
/// # Returns
/// Ok(()) if authentication succeeds, otherwise a SecurityScopeRejection
fn do_authenticate_security_scope<const N: usize>(
    required_rights: [AccessRight; N],
    request: &Request,
) -> Result<(), SecurityScopeRejection> {
    // Get entity from request:
    let entity = entity_from_request(request)?;

    // Get rights from db (if possible):
    let mut granted_rights: Vec<AccessRight> = entity
        .and_then(|entity| {
            readonly_tx(|tx| {
                Table::AccessRights.get(
                    tx,
                    bincode::serialize(&entity).expect("can serialize"),
                    |serialized| bincode::deserialize(serialized).unwrap(),
                )
            })
        })
        .unwrap_or_default();

    // Public is always granted, unconditionally.
    granted_rights.push(AccessRight::Public);

    // See if rights correspond to what is needed:
    if granted_rights
        .iter()
        .any(|right| required_rights.iter().any(|required| right >= required))
    {
        Ok(())
    } else {
        Err(SecurityScopeRejection::InsufficientPrivilege)
    }
}

/// Middelware that authenticates a call using the `Referer` header to extract the entity
/// and checking if the entity is a "trusted context" in the navigation of the site.
async fn authenticate_trusted_context(request: Request, next: Next) -> Response {
    let security_scope = do_authenticate_trusted_context(&request);
    let authorization = do_authenticate_authorization(&request);

    if let Some((security_scope_rejection, authorization_rejection)) =
        security_scope.err().zip(authorization.err())
    {
        return merge_rejections(security_scope_rejection, authorization_rejection);
    }

    next.run(request).await
}

/// Authenticates a call from a trusted context.
fn do_authenticate_trusted_context(request: &Request) -> Result<(), SecurityScopeRejection> {
    let Some(referer) = referer_from_request(request)? else {
        return Err(SecurityScopeRejection::MissingReferer);
    };

    if is_trusted_context(&referer) {
        check_origin(&referer).map_err(SecurityScopeRejection::BadOrigin)
    } else {
        Err(SecurityScopeRejection::NotTrustedContext(
            referer.to_owned(),
        ))
    }
}

/// Template for the registration page.
#[derive(askama::Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> {
    /// The entity requesting registration
    entity: &'a Entity,
    /// The rights being requested
    rights: &'a [AccessRight],
}

/// Query parameters for rights registration.
#[derive(Deserialize)]
struct RightsQuery {
    /// The access rights being requested
    right: Vec<AccessRight>,
}

/// Gets the registration page.
fn get_register() -> Router {
    Router::new().route(
        "/_register",
        get(
            |SecurityScope(entity): SecurityScope,
             AxumExtraQuery(RightsQuery { right }): AxumExtraQuery<RightsQuery>| {
                async move {
                    let register = RegisterTemplate {
                        entity: &entity,
                        rights: &right,
                    };

                    Html(register.render().expect("rendering _register failed"))
                }
            },
        ),
    )
}
