use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
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
pub struct QueryResponse {
    pub candidates: Vec<SocketAddr>,
}

#[tarpc::service]
pub trait Hub {
    /// Returns the port for the node to connect as server.
    async fn reverse_port() -> u16;
    /// Returns a greeting for name.
    async fn query(query: Query) -> QueryResponse;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Resolution {
    pub content_riddle: ContentRiddle,
    pub location_riddle: LocationRiddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolutionResponse {
    pub status: ResolutionStatus,
}

impl ResolutionResponse {
    pub const FOUND: ResolutionResponse = ResolutionResponse {
        status: ResolutionStatus::Found,
    };
    pub const NOT_FOUND: ResolutionResponse = ResolutionResponse {
        status: ResolutionStatus::NotFound,
    };
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ResolutionStatus {
    Found,
    NotFound,
}

#[tarpc::service]
pub trait Node {
    async fn resolve(resolution: Arc<Resolution>) -> ResolutionResponse;
}
