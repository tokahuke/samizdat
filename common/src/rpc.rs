use serde_derive::{Deserialize, Serialize};
use std::sync::Arc;

use crate::cipher::OpaqueEncrypted;
use crate::{ChannelAddr, Hash, MessageRiddle, Riddle};

pub type CandidateChannelId = u32;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum QueryKind {
    /// The hash corresponds to an object hash.
    Object,
    /// The hash corresponds to an item location (collection + path) hash.
    Item,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Query {
    /// The riddles the resolver can use to find the content hash.
    pub content_riddles: Vec<Riddle>,
    /// The riddle that will be used by the peer that has the hash to find the IP of the client.
    pub location_riddle: Riddle,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum QueryResponse {
    /// Hub experienced internal error (please report bug!)
    InternalError,
    /// Query was replayed. Therefore, it was rejected.
    Replayed,
    /// Query was empty, i.e., `content_riddles` was empty.
    EmptyQuery,
    /// You do not have a reverse connection to the hub (i.e. you are not connected as a server).
    NoReverseConnection,
    /// Query was run and returned and candidates may be following (watch `recv_candidate`).
    Resolved {
        candidate_channel: CandidateChannelId,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EditionRequest {
    pub key_riddle: Riddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EditionResponse {
    pub series: OpaqueEncrypted,
    pub rand: Hash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditionAnnouncement {
    pub key_riddle: Riddle,
    pub edition: OpaqueEncrypted,
    pub rand: Hash,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityRequest {
    pub identity_riddle: Riddle,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityResponse {
    pub identity: OpaqueEncrypted,
    pub rand: Hash,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityAnnouncement {}

#[tarpc::service]
pub trait Hub {
    /// Returns a response resolving (or not) the supplied object query.
    async fn query(query: Query) -> QueryResponse;
    /// Sends a candidate for a previously returned redirect for a resolution.
    async fn recv_candidate(candidate_channel: CandidateChannelId, candidate: Candidate);
    /// Gets the latest version of a series.
    async fn get_edition(request: EditionRequest) -> Vec<EditionResponse>;
    /// Announces a new edition of a series to the network.
    async fn announce_edition(announcement: EditionAnnouncement);
    /// Gets the series associated to a given identifier.
    async fn get_identity(request: IdentityRequest) -> Vec<IdentityResponse>;
    /// Announces a new identity to the network.
    async fn announce_identity(announcement: IdentityAnnouncement);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    /// The riddles the resolver can use to find the content hash.
    pub content_riddles: Vec<Riddle>,
    /// The nonces which the resolver must combine with the content hash to prove that it knows the
    /// correct hash.
    pub validation_nonces: Vec<Hash>,
    /// The riddle for the client address.
    pub location_message_riddle: MessageRiddle,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub channel_addr: ChannelAddr,
    pub validation_riddles: Vec<Riddle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ResolutionResponse {
    Found(Vec<Riddle>),
    Redirect(CandidateChannelId),
    EmptyResolution,
    NotFound,
}

#[tarpc::service]
pub trait Node {
    /// Tries to resolve an object or item query.
    async fn resolve(resolution: Arc<Resolution>) -> ResolutionResponse;
    /// Receives a candidate to start transferring the contents of a previously run query.
    async fn recv_candidate(candidate_channel: CandidateChannelId, candidate: Candidate);
    /// Tries to resolve the latest edition of a series.
    async fn get_edition(latest_request: Arc<EditionRequest>) -> Vec<EditionResponse>;
    /// Receives the announcement of a new edition.
    async fn announce_edition(announcement: Arc<EditionAnnouncement>);
    /// Tries to resolve an identity request.
    async fn get_identity(request: Arc<IdentityRequest>) -> Vec<IdentityResponse>;
    /// Receives the announcement of a new identity.
    async fn announce_identity(announcement: Arc<IdentityAnnouncement>);
}
