use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::{ContentRiddle, MessageRiddle};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum QueryKind {
    /// The hash corresponds to an object hash.
    Object,
    /// The hash corresponds to an item location (collection + path) hash.
    Item,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Query {
    /// The riddle that will be sent to the other peers.
    pub content_riddle: ContentRiddle,
    /// The riddle that will be used by the peer that has the hash to find the IP of the client.
    pub location_riddle: ContentRiddle,
    /// The kind of query.
    pub kind: QueryKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum QueryResponse {
    /// Server experienced internal error (please report bug!)
    InternalError,
    /// Query was replayed. Therefore, it was rejected.
    Replayed,
    /// Query was run and returned with these candidates (may be empty).
    Resolved { candidates: Vec<SocketAddr> },
}

#[tarpc::service]
pub trait Hub {
    /// Returns the port for the node to connect as server.
    async fn reverse_port() -> u16;
    /// Returns a response resolving (or not) the supplied object query.
    async fn query(query: Query) -> QueryResponse;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Resolution {
    pub content_riddle: ContentRiddle,
    pub message_riddle: MessageRiddle,
    pub kind: QueryKind,
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
    async fn resolve_object(resolution: Arc<Resolution>) -> ResolutionResponse;
    async fn resolve_item(resolution: Arc<Resolution>) -> ResolutionResponse;
}
