//! API call implementations for interacting with the Samizdat node.
//!
//! This module provides strongly-typed wrappers around HTTP endpoints exposed by the node,
//! organized into logical groups.

use anyhow::Context;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{str::FromStr, time::Duration};

use samizdat_common::{Hash, Key, Signed};

use super::{access_token, delete, get, patch, post, put, ApiError, CLIENT};

// Hubs:

/// Request parameters for registering a new hub.
#[derive(Debug, Serialize)]
pub struct PostHubRequest {
    /// Network address of the hub
    pub address: String,
    /// Mode used for resolving the DNS or socket address into an IP address.
    pub resolution_mode: String,
}

/// Response from registering a new hub.
#[derive(Debug, Deserialize)]
pub struct PostHubResponse {}

/// Registers a new hub with the node.
pub async fn post_hub(request: PostHubRequest) -> Result<PostHubResponse, anyhow::Error> {
    post("/_hubs", request).await
}

/// Response containing hub information.
#[derive(Debug, Deserialize)]
pub struct GetHubResponse {
    /// Network address of the hub
    pub address: String,
    /// Mode used for resolving the DNS or socket address into an IP address
    pub resolution_mode: String,
}

/// Retrieves all registered hubs.
pub async fn get_all_hubs() -> Result<Vec<GetHubResponse>, anyhow::Error> {
    get("/_hubs").await
}

/// Removes a hub from the node.
pub async fn delete_hub(address: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_hubs/{address}")).await
}

// Connections:

/// Response containing connection information.
#[derive(Debug, Deserialize)]
pub struct GetConnectionResponse {
    /// Name of the connection
    pub name: String,
    /// Current connection status
    pub status: String,
    /// Network address
    pub addr: String,
}

/// Retrieves all active connections.
pub async fn get_all_connections() -> Result<Vec<GetConnectionResponse>, anyhow::Error> {
    get("/_connections").await
}

// Peers:

/// Response containing peer information.
#[derive(Debug, Deserialize)]
pub struct GetPeerResponse {
    /// Network address of the peer
    pub addr: String,
    /// Current peer status
    pub status: String,
}

/// Retrieves all known peers.
pub async fn get_all_peers() -> Result<Vec<GetPeerResponse>, anyhow::Error> {
    get("/_peers").await
}

// Objects:

/// Posts a new object to the network.
///
/// # Arguments
/// * `content` - The raw bytes of the object to post
/// * `content_type` - The MIME type of the content being posted
/// * `bookmark` - Whether to bookmark this object so that it is not vacumed. away.
/// * `is_draft` - Whether this object is a draft version. Drafts are not exposed to the
/// network
///
/// # Returns
/// The hash of the posted object as a string
pub async fn post_object(
    content: Vec<u8>,
    content_type: &str,
    bookmark: bool,
    is_draft: bool,
) -> Result<String, anyhow::Error> {
    let url = format!("{}/_objects", crate::server()?);
    let response = CLIENT
        .post(format!(
            "{}/_objects?bookmark={}&is-draft={}",
            crate::server()?,
            bookmark,
            is_draft,
        ))
        .header("Content-Type", content_type)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .body(content)
        .send()
        .await
        .with_context(|| "error from samizdat-node request POST /_objects")?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| "error from samizdat-node response POST /_objects")?;

    tracing::info!("{} POST {} {}", status, url, text);

    let content: Result<String, ApiError> = serde_json::from_str(&text)
        .with_context(|| format!("error deserializing response from POST /_objects: {text}"))?;

    Ok(content?)
}

/// Retrieves an object from the network and executes a fallible callback for each chunk.
pub async fn get_object<F>(hash: &str, timeout: u64, mut each_chunk: F) -> Result<(), anyhow::Error>
where
    F: FnMut(Vec<u8>) -> Result<(), anyhow::Error>,
{
    let url = format!("{}/_objects/{}", crate::server()?, hash);
    let response = CLIENT
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token()?))
        .header("X-Samizdat-Timeout", timeout)
        .send()
        .await
        .with_context(|| "error from samizdat-node request POST /_objects")?;
    let status = response.status();
    tracing::info!("{} GET {}", status, url);

    if !status.is_success() {
        anyhow::bail!(
            "{}",
            response
                .text()
                .await
                .with_context(|| "error from samizdat-node response POST /_objects")?
        );
    } else {
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| {
                format!("receiving data chunk from samizdat-node request GET /_object/{hash}")
            })?;
            each_chunk(chunk.to_vec())?;
        }
    }

    Ok(())
}

// Series owners:

/// Key pair for a series owner.
#[derive(Debug, Serialize)]
pub struct Keypair {
    /// Public key of the pair
    pub public_key: String,
    /// Private key of the pair
    pub private_key: String,
}

/// Request parameters for creating a new series owner.
#[derive(Debug, Serialize)]
pub struct PostSeriesOwnerRequest<'a> {
    /// Name of the series owner
    pub series_owner_name: &'a str,
    /// Optional keypair for the series
    pub keypair: Option<Keypair>,
    /// Whether this is a draft series. Drafts are not exposed to the network
    pub is_draft: bool,
}

/// Response from creating a new series owner.
#[derive(Deserialize)]
pub struct PostSeriesOwnerResponse {
    /// Name of the series owner
    pub name: String,
    /// Keypair for signing editions
    pub keypair: ed25519_dalek::SigningKey,
    /// Default time-to-live for editions
    #[serde(with = "humantime_serde")]
    pub default_ttl: Duration,
}

/// Response containing series information.
#[derive(Deserialize)]
pub struct GetSeriesResponse {
    /// Public key of the series
    pub public_key: Key,
}

/// Retrieves all series.
pub async fn get_all_series() -> Result<Vec<GetSeriesResponse>, anyhow::Error> {
    get("/_series").await
}

/// Response containing edition information.
#[derive(Deserialize)]
pub struct GetEditionResponse {
    /// Signed edition content
    pub signed: Signed<EditionContent>,
    /// Public key of the series
    pub public_key: Key,
    /// Whether this is a draft edition
    #[serde(default)]
    pub is_draft: bool,
}

/// Retrieves all editions.
pub async fn get_all_editions() -> Result<Vec<GetEditionResponse>, anyhow::Error> {
    get("/_editions").await
}

/// Response containing series owner information.
type GetSeriesOwnerResponse = PostSeriesOwnerResponse;

/// Creates a new series owner.
pub async fn post_series_owner(
    request: PostSeriesOwnerRequest<'_>,
) -> Result<PostSeriesOwnerResponse, anyhow::Error> {
    post("/_series-owners", request).await
}

/// Removes a series owner.
pub async fn delete_series_owner(series_name: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_series-owners/{series_name}")).await
}

/// Retrieves a specific series owner.
pub async fn get_series_owner(series_name: &str) -> Result<GetSeriesOwnerResponse, anyhow::Error> {
    get(format!("/_series-owners/{series_name}")).await
}

/// Retrieves all series owners.
pub async fn get_all_series_owners() -> Result<Vec<GetSeriesOwnerResponse>, anyhow::Error> {
    get("/_series-owners").await
}

// Auth:

/// A name of an entity inside the Samizdat network. An entity can be an object, a
/// collection item, a series item, etc...
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    /// The type of the entity
    pub r#type: String,
    /// The identifier of the entity
    pub identifier: String,
}

/// Response containing authorization request information.
#[derive(Deserialize)]
pub struct GetAuthRequest {
    /// The entity being authorized
    pub entity: Entity,
    /// List of rights granted to the entity
    pub granted_rights: Vec<String>,
}

/// Retrieves all rights granted to all entities.
pub async fn get_auths() -> Result<Vec<GetAuthRequest>, anyhow::Error> {
    get("/_auth").await
}

/// Request parameters for updating authorization rights.
#[derive(Serialize)]
pub struct PatchAuthRequest {
    /// List of rights to be granted
    pub granted_rights: Vec<String>,
}

/// Updates authorization rights for a given scope.
pub async fn patch_auth(scope: &str, request: PatchAuthRequest) -> Result<bool, anyhow::Error> {
    patch(format!("/_auth/{scope}"), request).await
}

/// Removes authorization for a given scope.
pub async fn delete_auth(scope: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_auth/{scope}")).await
}

// Collections:

/// Request parameters for creating a new collection.
#[derive(Debug, Serialize)]
pub struct PostCollectionRequest<'a> {
    /// Definition of the collection. The first element if the path of the item, the
    /// second is the hash of the item.
    pub hashes: &'a [(String, String)],
    /// Whether this is a draft collection. Drafts are not exposed to the network.
    pub is_draft: bool,
}

#[allow(unused)]
pub async fn post_collection(request: PostCollectionRequest<'_>) -> Result<String, anyhow::Error> {
    post("/_collections", request).await
}

/// Retrieves the list of items in a collection.
pub async fn get_collection_list(collection: &str) -> Result<Vec<String>, anyhow::Error> {
    get(format!("/_collections/{collection}/_list")).await
}

// Subscriptions:

/// Request parameters for creating a new subscription.
#[derive(Debug, Serialize)]
pub struct PostSubscriptionRequest<'a> {
    /// Public key to subscribe to
    pub public_key: &'a str,
}

/// Response containing subscription information.
#[derive(Debug, Deserialize)]
pub struct GetSubscriptionResponse {
    /// Public key of the subscription
    pub public_key: Key,
    /// Type of subscription
    pub kind: String,
}

/// Creates a new subscription.
pub async fn post_subscription(
    request: PostSubscriptionRequest<'_>,
) -> Result<String, anyhow::Error> {
    post("/_subscriptions", request).await
}

/// Refreshes a subscription for a given public key.
pub async fn get_subscription_refresh(public_key: &str) -> Result<(), anyhow::Error> {
    get(format!("/_subscriptions/{public_key}/refresh")).await
}

/// Removes a subscription for a given public key.
pub async fn delete_subscription(public_key: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_subscriptions/{public_key}")).await
}

/// Retrieves all active subscriptions.
pub async fn get_all_subscriptions() -> Result<Vec<GetSubscriptionResponse>, anyhow::Error> {
    get("/_subscriptions").await
}

// Editions:

/// The kind of an edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EditionKind {
    /// Forget everything that came before. All the content will start from scratch.
    Base,
    /// Add to what came before. If an item is not found in the current edition, search
    /// for the content in previous editions (unless _explicitely deleted_).
    Layer,
}

impl FromStr for EditionKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "base" => Ok(EditionKind::Base),
            "layer" => Ok(EditionKind::Layer),
            oops => anyhow::bail!("Edition kind must be either `base` or `layer`, got {oops}"),
        }
    }
}

/// Request parameters for creating a new edition.
#[derive(Debug, Serialize)]
pub struct PostEditionRequest<'a> {
    /// Type of edition
    pub kind: EditionKind,
    /// Time-to-live duration
    pub ttl: Option<&'a str>,
    /// Whether to skip announcing the edition
    pub no_announce: bool,
    /// Whether this is a draft edition
    pub is_draft: bool,
    /// List of hash pairs defining the edition content
    pub hashes: &'a [(String, String)],
}

/// Reference to a collection.
#[derive(Debug, Clone, Deserialize)]
pub struct CollectionRef {
    /// Hash of the collection
    pub hash: Hash,
}

/// Content of an edition.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct EditionContent {
    /// Type of edition
    pub kind: EditionKind,
    /// Hash of the collection
    pub collection: CollectionRef,
    /// Creation timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Time-to-live duration
    #[serde(with = "humantime_serde")]
    pub ttl: Duration,
}

/// Response from creating a new edition.
#[derive(Debug, Deserialize)]
pub struct PostEditionResponse {
    /// Signed edition content
    pub signed: Signed<EditionContent>,
}

/// Creates a new edition for a series.
pub async fn post_edition(
    series_name: &str,
    request: PostEditionRequest<'_>,
) -> Result<PostEditionResponse, anyhow::Error> {
    post(format!("/_series-owners/{series_name}/editions",), request).await
}

/// Reference to a series.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct SeriesRef {
    /// Public key of the series
    pub public_key: Key,
}

// Identities:

/// Response from updating the Polygon provider.
#[derive(Debug, Deserialize)]
pub struct PutPolygonProviderResponse {}

/// Updates the Polygon provider endpoint.
pub async fn put_polygon_provider(
    endpoint: String,
) -> Result<PutPolygonProviderResponse, anyhow::Error> {
    put("/_polygon-provider", GetPolygonProvider { endpoint }).await
}

/// Configuration for a Polygon provider.
#[derive(Debug, Serialize, Deserialize)]
pub struct GetPolygonProvider {
    /// Endpoint URL for the provider
    pub endpoint: String,
}

/// Retrieves the current Polygon provider configuration.
pub async fn get_polygon_provider() -> Result<GetPolygonProvider, anyhow::Error> {
    get("/_polygon-provider").await
}

// Vacuum:

/// Status for a vacuum task.
#[derive(Debug, Serialize, Deserialize)]
pub enum VacuumStatus {
    /// Storage is within allowed parameters.
    Unnecessary,
    /// Removed all disposable content, but could not achieve the desired maximum size.
    Insufficient,
    /// Storage has run and was able to reduce the storage size.
    Done,
}

/// Triggers a new vacuum round.
pub async fn post_vacuum() -> Result<VacuumStatus, anyhow::Error> {
    post("/_vacuum", ()).await
}

/// Flushes the whole Samizdat cache.
pub async fn post_flush_all() -> Result<(), anyhow::Error> {
    post("/_vacuum/flush-all", ()).await
}
