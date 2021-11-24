use askama::Template;
use lazy_static::lazy_static;
use rocksdb::IteratorMode;
use serde_derive::Deserialize;
use std::fmt::{self, Display};
use url::Url;
use warp::Filter;

use crate::access_token::{access_token, AccessRight, Entity};
use crate::balanced_or_tree;
use crate::cli;
use crate::db::{db, Table};

use super::{api_reply, html};

/// The authrntication management API.
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
        .and(warp::header("Referer"))
        .map(|referer: String| {
            let maybe_entity = entity_from_referer(&referer).map_err(|err| {
                crate::Error::Message(format!("Referer {} not an entity: {}", referer, err))
            })?;

            if let Some(entity) = maybe_entity {
                let serialized = bincode::serialize(&entity).expect("can serialize");
                let current: Vec<AccessRight> = db()
                    .get_cf(Table::AccessRights.get(), &serialized)?
                    .map(|rights| bincode::deserialize(&rights))
                    .transpose()?
                    .unwrap_or_default();

                Ok(current)
            } else {
                Err(crate::Error::Message(
                    "Accessing /_auth/_current from trusted context".to_owned(),
                ))
            }
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
    InsuficientPrivilege,
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
            Forbidden::InsuficientPrivilege => write!(f, "insuficient privilege"),
        }
    }
}

/// Authentication header was not sent and must be sent.
#[derive(Debug, Clone, Copy)]
pub struct Unauthorized;

impl warp::reject::Reject for Unauthorized {}

lazy_static! {
    static ref SAMIZDA_ORIGINS: [url::Origin; 4] = [
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

/// Checks whether the request is _realy_ comming from Samizdat. This is a
/// complement to CORS.
fn check_origin(referer: &Url) -> Result<(), Forbidden> {
    let origin = referer.origin();

    // Find out if some cross-origin thing is trying ot trick you.
    // TODO: also implement proper CORS.
    if !SAMIZDA_ORIGINS.contains(&origin) {
        Err(Forbidden::BadOrigin(origin))
    } else {
        Ok(())
    }
}

/// Paths which are *always* trusted.
fn is_trusted_context(referer: &Url) -> bool {
    ["/_register"].contains(&referer.path())
}

/// Returns `Ok(None)` when trusted context.
fn entity_from_referer(raw_referer: &str) -> Result<Option<Entity>, Forbidden> {
    let referer: Url = raw_referer.parse().unwrap();

    check_origin(&referer)?;

    if is_trusted_context(&referer) {
        Ok(None)
    } else {
        // Find which entity is requesting authorization.
        Ok(Some(Entity::from_path(referer.path()).ok_or_else(
            || Forbidden::NotAnEntity(referer.path().to_owned()),
        )?))
    }
}

/// Authenticate to the private part of the API. This requires access to the filesystem (only that)
pub fn authenticate<const N: usize>(
    required_rights: [AccessRight; N],
) -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    warp::header::optional("Authorization")
        .and(warp::header::optional("Referer"))
        .map(
            move |authorization: Option<String>, referer: Option<String>| match (
                authorization,
                referer,
            ) {
                (None, None) => Err(warp::reject::custom(Unauthorized)),
                (Some(authorization), _) => {
                    let token = authorization
                        .trim_start_matches("Bearer ")
                        .trim_start_matches("bearer ");

                    if token == access_token() {
                        Ok(())
                    } else {
                        Err(warp::reject::custom(Forbidden::BadToken(token.to_owned())))
                    }
                }
                (_, Some(referer)) => {
                    let maybe_entity = entity_from_referer(&referer)
                        .map_err(|forbidden| warp::reject::custom(forbidden))?;

                    if let Some(entity) = maybe_entity {
                        // Get rights from db:
                        let serialized = db()
                            .get_cf(
                                Table::AccessRights.get(),
                                bincode::serialize(&entity).expect("can serialize"),
                            )
                            .unwrap()
                            .ok_or_else(|| warp::reject::custom(Forbidden::InsuficientPrivilege))?;
                        let granted_rights: Vec<AccessRight> =
                            bincode::deserialize(&serialized).unwrap();

                        // See if rights correspond to what is needed:
                        if granted_rights
                            .iter()
                            .any(|right| required_rights.contains(right))
                        {
                            Ok(())
                        } else {
                            Err(warp::reject::custom(Forbidden::InsuficientPrivilege))
                        }
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .and_then(|auth| async move { auth })
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
        .and(warp::header("Referer"))
        .and(warp::query())
        .map(|referer: String, query: Vec<(String, AccessRight)>| {
            let maybe_entity = entity_from_referer(&referer)
                .map_err(|forbidden| warp::reject::custom(forbidden))?;

            if let Some(entity) = maybe_entity {
                let register = RegisterTemplate {
                    entity: &entity,
                    rights: &*query
                        .into_iter()
                        .filter(|(field, _)| field == "right")
                        .map(|(_, right)| right)
                        .collect::<Vec<_>>(),
                };

                Ok(html(register.render().expect("rendering _register failed")))
            } else {
                Err(warp::reject::custom(crate::Error::Message(
                    "Accessing /_register from trusted context".to_owned(),
                )))
            }
        })
        .and_then(|auth: Result<_, warp::Rejection>| async move { auth })
}
