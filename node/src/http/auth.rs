use askama::Template;
use lazy_static::lazy_static;
use rocksdb::IteratorMode;
use serde_derive::Deserialize;
use std::fmt::{self, Display};
use url::Url;
use warp::Filter;

use crate::access::{access_token, AccessRight, Entity};
use crate::balanced_or_tree;
use crate::cli;
use crate::db::{db, Table};

use super::{api_reply, html};

/// The authentication management API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        get_auth_current(),
        get_auth(),
        get_auths(),
        patch_auth(),
        delete_auth(),
        get_register(),
    )
}

fn get_auth() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_auth" / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and(authenticate([]))
        .map(|tail: warp::path::Tail| {
            let entity = Entity::from_path(tail.as_str()).ok_or_else(|| "not an entity")?;
            let serialized = bincode::serialize(&entity).expect("can serialize");
            let current: Vec<AccessRight> = db()
                .get_cf(Table::AccessRights.get(), &serialized)?
                .map(|rights| bincode::deserialize(&rights))
                .transpose()?
                .unwrap_or_default();

            Ok(current) as Result<_, crate::Error>
        })
        .map(api_reply)
}

fn get_auth_current() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone
{
    warp::path!("_auth" / "_current")
        .and(warp::get())
        .and(security_scope())
        .map(|entity: Entity| {
            let serialized = bincode::serialize(&entity).expect("can serialize");
            let current: Vec<AccessRight> = db()
                .get_cf(Table::AccessRights.get(), &serialized)?
                .map(|rights| bincode::deserialize(&rights))
                .transpose()?
                .unwrap_or_default();

            Ok(current)
        })
        .map(api_reply)
}

fn get_auths() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_auth")
        .and(warp::get())
        .and(authenticate([]))
        .map(|| {
            let all_auths = db()
                .iterator_cf(Table::AccessRights.get(), IteratorMode::Start)
                .map(|(key, value)| {
                    let entity = bincode::deserialize(&key)?;
                    let granted_rights = bincode::deserialize(&value)?;

                    Ok((entity, granted_rights))
                })
                .collect::<Result<Vec<_>, crate::Error>>()?;

            Ok(all_auths) as Result<_, crate::Error>
        })
        .map(api_reply)
}

fn patch_auth() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Debug, Deserialize)]
    struct Request {
        granted_rights: Vec<AccessRight>,
    }

    warp::path!("_auth" / ..)
        .and(warp::path::tail())
        .and(warp::patch())
        .and(authenticate([]))
        .and(warp::body::json())
        .map(|tail: warp::path::Tail, request: Request| {
            let entity = Entity::from_path(tail.as_str()).ok_or_else(|| "not an entity")?;
            let serialized = bincode::serialize(&entity).expect("can serialize");
            let mut current: Vec<AccessRight> = db()
                .get_cf(Table::AccessRights.get(), &serialized)?
                .map(|rights| bincode::deserialize(&rights))
                .transpose()?
                .unwrap_or_default();

            current.extend(request.granted_rights);
            current.sort_unstable();
            current.dedup();

            db().put_cf(
                Table::AccessRights.get(),
                &serialized,
                bincode::serialize(&current).expect("can serialize"),
            )?;

            Ok(()) as Result<_, crate::Error>
        })
        .map(api_reply)
}

fn delete_auth() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_auth" / ..)
        .and(warp::path::tail())
        .and(warp::delete())
        .and(authenticate([]))
        .map(|tail: warp::path::Tail| {
            let entity = Entity::from_path(tail.as_str()).ok_or_else(|| "not an entity")?;
            db().delete_cf(
                Table::AccessRights.get(),
                bincode::serialize(&entity).expect("can serialize"),
            )?;

            Ok(()) as Result<(), crate::Error>
        })
        .map(api_reply)
}

/// Authentication header was sent, but token was invalid.
#[derive(Debug)]
pub enum Forbidden {
    BadToken(String),
    BadOrigin(url::Origin),
    NotAnEntity(String),
    TrustedContext(String),
    InsufficientPrivilege,
}

impl warp::reject::Reject for Forbidden {}

impl Display for Forbidden {
    fn fmt(&self, f: &'_ mut fmt::Formatter) -> fmt::Result {
        match self {
            Forbidden::BadToken(token) => write!(f, "bad token: {}", token),
            Forbidden::BadOrigin(origin) => {
                write!(f, "bad origin: {}", origin.unicode_serialization())
            }
            Forbidden::NotAnEntity(bad_path) => write!(f, "not an entity: {}", bad_path),
            Forbidden::TrustedContext(context) => {
                write!(f, "accessing from trusted context {}", context)
            }
            Forbidden::InsufficientPrivilege => write!(f, "insufficient privilege"),
        }
    }
}

/// Authentication header was not sent and must be sent.
#[derive(Debug, Clone, Copy)]
pub enum Unauthorized {
    MissingReferrer,
    Unauthorized,
}

impl warp::reject::Reject for Unauthorized {}

impl Display for Unauthorized {
    fn fmt(&self, f: &'_ mut fmt::Formatter) -> fmt::Result {
        match self {
            Unauthorized::MissingReferrer => write!(f, "missing Referrer header"),
            Unauthorized::Unauthorized => write!(f, "missing Referrer header or Bearer token"),
        }
    }
}

lazy_static! {
    static ref SAMIZDAT_ORIGINS: [url::Origin; 4] = [
        url::Origin::Tuple(
            "http".to_owned(),
            url::Host::Domain("localhost".to_owned()),
            cli().port,
        ),
        url::Origin::Tuple(
            "http".to_owned(),
            url::Host::Ipv4([127, 0, 0, 1].into()),
            cli().port,
        ),
        url::Origin::Tuple(
            "http".to_owned(),
            url::Host::Ipv4([0, 0, 0, 0].into()),
            cli().port,
        ),
        url::Origin::Tuple(
            "http".to_owned(),
            url::Host::Ipv6([0; 16].into()),
            cli().port,
        ),
    ];
}

/// Checks whether the request is _really_ coming from Samizdat. This is a
/// complement to CORS.
fn check_origin(referrer: &Url) -> Result<(), Forbidden> {
    let origin = referrer.origin();

    // Find out if some cross-origin thing is trying ot trick you.
    // TODO: also implement proper CORS.
    if !SAMIZDAT_ORIGINS.contains(&origin) {
        Err(Forbidden::BadOrigin(origin))
    } else {
        Ok(())
    }
}

/// Paths which are *always* trusted.
fn is_trusted_context(referrer: &Url) -> bool {
    ["/_register"].contains(&referrer.path())
}

/// Returns `Ok(None)` when trusted context.
fn entity_from_referrer(raw_referrer: &str) -> Result<Option<Entity>, Forbidden> {
    let referrer: Url = raw_referrer.parse().unwrap();

    check_origin(&referrer)?;

    if is_trusted_context(&referrer) {
        Ok(None)
    } else {
        // Find which entity is requesting authorization.
        Ok(Some(Entity::from_path(referrer.path()).ok_or_else(
            || Forbidden::NotAnEntity(referrer.path().to_owned()),
        )?))
    }
}

/// Extracts the "security scope" (akin to "origin" in the normal Web) from the Referrer header.
pub fn security_scope() -> impl Filter<Extract = (Entity,), Error = warp::Rejection> + Clone {
    warp::header::optional("Referrer").and_then(|maybe_referrer: Option<String>| async move {
        let referrer =
            maybe_referrer.ok_or_else(|| warp::reject::custom(Unauthorized::MissingReferrer))?;
        let maybe_entity =
            entity_from_referrer(&referrer).map_err(|forbidden| warp::reject::custom(forbidden))?;
        let entity = maybe_entity
            .ok_or_else(|| warp::reject::custom(Forbidden::TrustedContext(referrer)))?;

        Ok(entity) as Result<Entity, warp::Rejection>
    })
}

/// Authenticate to the private part of the API. This requires access to the filesystem (only that)
pub fn authenticate<const N: usize>(
    required_rights: [AccessRight; N],
) -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    let authorization_access = warp::header("Authorization")
        .or_else(|_| async { Err(warp::reject::custom(Unauthorized::Unauthorized)) })
        .map(|authorization: String| {
            let token = authorization
                .trim_start_matches("Bearer ")
                .trim_start_matches("bearer ");

            if token == access_token() {
                None
            } else {
                Some(Forbidden::BadToken(token.to_owned()))
            }
        });

    let secure_scope_access = security_scope().map(move |entity: Entity| {
        // Get rights from db:
        let serialized_opt = db()
            .get_cf(
                Table::AccessRights.get(),
                bincode::serialize(&entity).expect("can serialize"),
            )
            .unwrap();
        let serialized = if let Some(serialized) = serialized_opt {
            serialized
        } else {
            return Some(Forbidden::InsufficientPrivilege);
        };

        let granted_rights: Vec<AccessRight> = bincode::deserialize(&serialized).unwrap();

        // See if rights correspond to what is needed:
        if granted_rights
            .iter()
            .any(|right| required_rights.contains(right))
        {
            None
        } else {
            Some(Forbidden::InsufficientPrivilege)
        }
    });

    authorization_access
        .or(secure_scope_access)
        .unify()
        .and_then(|outcome| async move {
            match outcome {
                None => Ok(()),
                Some(forbidden) => Err(warp::reject::custom(forbidden)),
            }
        })
        .untuple_one()
}

#[derive(askama::Template)]
#[template(path = "register.html")]
struct RegisterTemplate<'a> {
    entity: &'a Entity,
    rights: &'a [AccessRight],
}

fn get_register() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_register")
        .and(security_scope())
        .and(warp::query())
        .map(|entity: Entity, query: Vec<(String, AccessRight)>| {
            let register = RegisterTemplate {
                entity: &entity,
                rights: &*query
                    .into_iter()
                    .filter(|(field, _)| field == "right")
                    .map(|(_, right)| right)
                    .collect::<Vec<_>>(),
            };

            Ok(html(register.render().expect("rendering _register failed")))
        })
        .and_then(|auth: Result<_, warp::Rejection>| async move { auth })
}
