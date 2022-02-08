use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::cipher::OpaqueEncrypted;
use crate::{ChannelAddr, Hash, MessageRiddle, Riddle};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum QueryKind {
    /// The hash corresponds to an object hash.
    Object,
    /// The hash corresponds to an item location (collection + path) hash.
    Item,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Query {
    /// The riddle for the content hash.
    pub content_riddle: Riddle,
    /// The riddle that will be used by the peer that has the hash to find the IP of the client.
    pub location_riddle: Riddle,
    /// The riddle that will be used by the hub to validate if the node has *really* found the
    /// hash.
    pub validation_riddle: Riddle,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum QueryResponse {
    /// Hub experienced internal error (please report bug!)
    InternalError,
    /// Query was replayed. Therefore, it was rejected.
    Replayed,
    /// Query was run and returned with these candidates (may be empty).
    Resolved { candidates: Vec<ChannelAddr> },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatestRequest {
    pub key_riddle: Riddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LatestResponse {
    pub series: OpaqueEncrypted,
    pub rand: Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditionAnnouncement {
    pub key_riddle: Riddle,
    pub edition: OpaqueEncrypted,
    pub rand: Hash,
}

#[tarpc::service]
pub trait Hub {
    /// Returns a response resolving (or not) the supplied object query.
    async fn query(query: Query) -> QueryResponse;
    /// Gets the latest version of a series.
    async fn get_latest(latest: LatestRequest) -> Vec<LatestResponse>;
    /// Announces a new edition of a series to the network.
    async fn announce_edition(announcement: EditionAnnouncement);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Resolution {
    /// The riddle for the requested hash.
    pub content_riddle: Riddle,
    /// The riddle for the client address.
    pub message_riddle: MessageRiddle,
    /// The nonce which the node uses to prove to the hub that it know the correct hash.
    pub validation_nonce: Hash,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub peer_addr: SocketAddr,
    pub validation_riddle: Riddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ResolutionResponse {
    Found(Riddle),
    Redirect(Vec<Candidate>),
    NotFound,
}

#[tarpc::service]
pub trait Node {
    async fn resolve(resolution: Arc<Resolution>) -> ResolutionResponse;
    async fn resolve_latest(latest_request: Arc<LatestRequest>) -> Vec<LatestResponse>;
    async fn announce_edition(announcement: Arc<EditionAnnouncement>);
}
