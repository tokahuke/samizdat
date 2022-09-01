use serde_derive::Deserialize;
use warp::path::Tail;
use warp::Filter;

use samizdat_common::pow::ProofOfWork;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::db;
use crate::models::{Identity, IdentityRef};

use super::resolvers::resolve_identity;
use super::{api_reply, authenticate, tuple};

/// The entrypoint of the object API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // // Identity CRUD
        // get_identity(),
        get_identities(),
        post_identity(),
        // delete_identity(),
        // Query item using identity
        get_item(),
    )
}

/// Gets the contents of an item using identity.
fn get_item() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!(IdentityRef / ..)
        .and(warp::path::tail())
        .and(warp::get())
        .and_then(|identity, name: Tail| async move {
            Ok(resolve_identity(identity, name.as_str().into(), []).await?)
                as Result<_, warp::Rejection>
        })
        .map(tuple)
}

fn post_identity() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[derive(Deserialize)]
    struct Request {
        identity: String,
        series: String,
        proof: ProofOfWork,
    }

    warp::path!("_identities")
        .and(warp::post())
        .and(authenticate([AccessRight::ManageIdentities]))
        .and(warp::body::json())
        .map(|request: Request| {
            let identity = Identity {
                identity: request.identity.parse()?,
                series: request.series.parse()?,
                proof: request.proof,
            };

            if identity.is_valid() {
                let existing_work_done = if let Some(existing) = Identity::get(&identity.identity)?
                {
                    existing.work_done()
                } else {
                    0.0
                };

                if identity.work_done() > existing_work_done {
                    let mut batch = rocksdb::WriteBatch::default();
                    identity.insert(&mut batch);
                    db().write(batch)?;

                    Ok(true)
                } else {
                    Ok(false)
                }
            } else {
                Err(crate::Error::Message(format!(
                    "Invalid identity: {identity:?}"
                )))
            }
        })
        .map(api_reply)
}

fn get_identities() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_identities")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageIdentities]))
        .map(|| Identity::get_all())
        .map(api_reply)
}

// fn delete_identity() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
//     warp::path!("_identities" / IdentityRef)
//         .and(warp::delete())
//         .and(authenticate([AccessRight::ManageIdentities]))
//         .map(|identity| {

//         }).map(api_reply)
// }
