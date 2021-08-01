use serde_derive::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{ContentRiddle, LocationRiddle};

#[derive(Debug, Serialize, Deserialize)]
pub struct Query {
    /// The riddle that will be sent to the other peers.
    pub content_riddle: ContentRiddle,
    /// The riddle that will be used by the peer that has the hash to find the IP of the client.
    pub location_riddle: ContentRiddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Resolution {
    pub content_riddle: ContentRiddle,
    pub location_riddle: LocationRiddle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Rejected,
    NotFound,
    Found,
}

#[tarpc::service]
pub trait Hub {
    /// Returns a greeting for name.
    async fn query(query: Query) -> Status;
}

#[tarpc::service]
pub trait Node {
    async fn resolve(resolution: Arc<Resolution>) -> Status;
}
