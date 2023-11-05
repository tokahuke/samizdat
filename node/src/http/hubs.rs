//! Hubs API.

use samizdat_common::address::AddrResolutionMode;
use samizdat_common::address::AddrToResolve;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_with::serde_as;
use serde_with::DisplayFromStr;
use warp::Filter;

use crate::access::AccessRight;
use crate::balanced_or_tree;
use crate::models::Droppable;
use crate::models::Hub;

use super::{api_reply, authenticate};

/// The entrypoint of the hub API.
pub fn api() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    balanced_or_tree!(
        // Hub CRUD
        post_hub(),
        get_hub(),
        get_hubs(),
        delete_hub(),
    )
}

fn post_hub() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    #[serde_as]
    #[derive(Deserialize)]
    struct Request {
        #[serde_as(as = "DisplayFromStr")]
        address: AddrToResolve,
        resolution_mode: AddrResolutionMode,
    }

    #[derive(Serialize)]
    struct Response {}

    warp::path!("_hubs")
        .and(warp::post())
        .and(authenticate([AccessRight::ManageHubs]))
        .and(warp::body::json())
        .map(|request: Request| {
            let hub = Hub {
                address: request.address,
                resolution_mode: request.resolution_mode,
            };

            hub.insert()?;

            Ok(Response {})
        })
        .map(api_reply)
}

/// Lists all hubs.
fn get_hub() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_hubs" / AddrToResolve)
        .and(warp::get())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|address| Hub::get(address))
        .map(api_reply)
}

/// Lists all hubs.
fn get_hubs() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_hubs")
        .and(warp::get())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|| Hub::get_all())
        .map(api_reply)
}

fn delete_hub() -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    warp::path!("_hubs" / AddrToResolve)
        .and(warp::delete())
        .and(authenticate([AccessRight::ManageHubs]))
        .map(|address: AddrToResolve| {
            let existed = if let Some(hub) = Hub::get(address)? {
                hub.drop_if_exists()?;
                true
            } else {
                false
            };

            Ok(existed)
        })
        .map(api_reply)
}
