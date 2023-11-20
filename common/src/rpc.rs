//! The Samizdat RPC definition using [`tarpc`].

use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::address::ChannelId;
use crate::cipher::OpaqueEncrypted;
use crate::riddles::Hint;
use crate::{Hash, MessageRiddle, Riddle};

#[derive(Debug, Serialize, Deserialize)]
pub enum SetPropertyResponse {
    /// The new value is now in place.
    Set,
    /// The requested property is not supported by the hub.
    Unsupported,
    /// The set property request is invalid, i.e., a _node side_ error has occurred.
    Error(String),
}

/// The kind of a query, i.e., whether the sent content hash corresponds to an object
/// hash or to an item hash.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum QueryKind {
    /// The hash corresponds to an object hash.
    Object,
    /// The hash corresponds to an item location (collection + path) hash.
    Item,
}

/// A query of a given information.
#[derive(Debug, Serialize, Deserialize)]
pub struct Query {
    /// The riddles the resolver can use to find the content hash.
    pub content_riddles: Vec<Riddle>,
    /// A hint to the solution of the content hash. This can set the degree of toughness of solving
    /// the content riddles.
    pub hint: Hint,
    /// The riddle that will be used by the peer that has the hash to find the IP of the client.
    pub location_riddle: Riddle,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

/// A response to a given query, given by a node to a hub.
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
        /// The id of the channel through which the candidates will arrive.
        candidate_channel: ChannelId,
        /// The channel to be used to to transport the payload.
        channel_id: ChannelId,
    },
}

/// A request for an edition with key defined by a riddle.
#[derive(Debug, Serialize, Deserialize)]
pub struct EditionRequest {
    /// The riddle defining the series public key.
    pub key_riddle: Riddle,
}

/// The response to an edition request.
#[derive(Debug, Serialize, Deserialize)]
pub struct EditionResponse {
    /// The series corresponding to that edition.
    pub series: OpaqueEncrypted,
    /// The random initialization to be used when decoding the encrypted value of
    /// `series`.
    pub rand: Hash,
}

/// An announcement of a new edition in a series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditionAnnouncement {
    /// The riddle defining the series public key.
    pub key_riddle: Riddle,
    /// The information for the new edition.
    pub edition: OpaqueEncrypted,
    /// The random initialization to be used when decoding the encrypted value of
    /// `edition`.
    pub rand: Hash,
}

/// A request for an identity within the network.
#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityRequest {
    /// The riddle corresponding to the identity name.
    pub identity_riddle: Riddle,
}

/// The response for an identity request.
#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityResponse {
    /// The identity information (incl. the corresponding series public key).
    pub identity: OpaqueEncrypted,
    /// The random initialization to be used when decoding the encrypted value of
    /// `identity`.
    pub rand: Hash,
}

/// The announcement of a new identity candidate in the network (as of yet, _unimplemented_).
#[derive(Debug, Serialize, Deserialize)]
pub struct IdentityAnnouncement {}

/// The Samizdat hub RPC interface.
#[tarpc::service]
pub trait Hub {
    /// Sets a property for the current connection.
    async fn set_property(key: String, value: serde_json::Value) -> SetPropertyResponse;
    /// Returns a response resolving (or not) the supplied object query.
    async fn query(query: Query) -> QueryResponse;
    /// Sends a candidate for a previously returned redirect for a resolution.
    async fn recv_candidate(candidate_channel: ChannelId, candidate: Candidate);
    /// Gets the latest version of a series.
    async fn get_edition(request: EditionRequest) -> Vec<EditionResponse>;
    /// Announces a new edition of a series to the network.
    async fn announce_edition(announcement: EditionAnnouncement);
}

/// A resolution to a given query, given by the hub to a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    /// The riddles the resolver can use to find the content hash.
    pub content_riddles: Vec<Riddle>,
    /// The nonces which the resolver must combine with the content hash to prove that it knows the
    /// correct hash.
    pub validation_nonces: Vec<Hash>,
    /// A hint to the solution of the content hash. This can set the degree of toughness of solving
    /// the content riddles.
    pub hint: Hint,
    /// The riddle for the client address.
    pub location_message_riddle: MessageRiddle,
    /// The kind of entity being requested.
    pub kind: QueryKind,
}

/// A promise of a possible peer in the network that might know the answer to a given query.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    /// The address of the peer.
    pub socket_addr: SocketAddr,
    /// The validation riddles that the peer has supplied.
    pub validation_riddles: Vec<Riddle>,
}

/// The response of a node to a hub on the resolution status of a query.
#[derive(Debug, Serialize, Deserialize)]
pub enum ResolutionResponse {
    Found(Vec<Riddle>),
    Redirect(ChannelId),
    EmptyResolution,
    NotFound,
}

/// The Samizdat node RPC interface.
#[tarpc::service]
pub trait Node {
    /// Tries to resolve an object or item query.
    async fn resolve(resolution: Arc<Resolution>) -> ResolutionResponse;
    /// Receives a candidate to start transferring the contents of a previously run query.
    async fn recv_candidate(candidate_channel: ChannelId, candidate: Candidate);
    /// Tries to resolve the latest edition of a series.
    async fn get_edition(latest_request: Arc<EditionRequest>) -> Vec<EditionResponse>;
    /// Receives the announcement of a new edition.
    async fn announce_edition(announcement: Arc<EditionAnnouncement>);
}
