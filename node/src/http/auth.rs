use rocksdb::IteratorMode;
use serde_derive::Deserialize;
use warp::Filter;
use std::fmt::{self, Display};
use url::Url;

use crate::access_token::{AccessRight, Entity, access_token};
use crate::db::{db, Table};
use crate::balanced_or_tree;
use crate::cli;

use super::{reply, Json};

/// The authrntication management API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        get_auth(),
        get_auths(),
        patch_auth(),
        delete_auth(),
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

            Ok(Json(current)) as Result<_, crate::Error>
        })
        .map(reply)
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

            Ok(Json(all_auths)) as Result<_, crate::Error>
        })
        .map(reply)
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
        .map(reply)
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
        .map(reply)
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

/// Authenticate to the private part of the API. This requires access to the filesystem (only that)
pub fn authenticate<const N: usize>(
    required_rights: [AccessRight; N],
) -> impl Filter<Extract = (), Error = warp::Rejection> + Clone {
    let samizdat_origins = [
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

    warp::header::optional("Authorization")
        .and(warp::header::optional("Referrer"))
        .map(
            move |authorization: Option<String>, referrer: Option<String>| match (
                authorization,
                referrer,
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
                (_, Some(referrer)) => {
                    // Need the referrer to judge if page can access API route.
                    let referrer: Url = referrer.parse().unwrap();
                    let origin = referrer.origin();

                    // Find out if some cross-origin thing is trying ot trick you.
                    // TODO: also implement proper CORS.
                    if !samizdat_origins.contains(&origin) {
                        return Err(warp::reject::custom(Forbidden::BadOrigin(origin)));
                    }

                    // Find which entity is requesting authorization.
                    let entity = Entity::from_path(referrer.path()).ok_or_else(|| {
                        warp::reject::custom(Forbidden::NotAnEntity(referrer.path().to_owned()))
                    })?;

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
                }
            },
        )
        .and_then(|auth| async move { auth })
        .untuple_one()
}
