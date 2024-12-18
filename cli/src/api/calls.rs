use anyhow::Context;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::{str::FromStr, time::Duration};

use samizdat_common::{Hash, Key, Signed};

use super::{access_token, delete, get, patch, post, put, ApiError, CLIENT};

// Hubs:

#[derive(Debug, Serialize)]
pub struct PostHubRequest {
    pub address: String,
    pub resolution_mode: String,
}

#[derive(Debug, Deserialize)]
pub struct PostHubResponse {}

pub async fn post_hub(request: PostHubRequest) -> Result<PostHubResponse, anyhow::Error> {
    post("/_hubs", request).await
}

#[derive(Debug, Deserialize)]
pub struct GetHubResponse {
    pub address: String,
    pub resolution_mode: String,
}

pub async fn get_all_hubs() -> Result<Vec<GetHubResponse>, anyhow::Error> {
    get("/_hubs").await
}

pub async fn delete_hub(address: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_hubs/{address}")).await
}

// Connections:

#[derive(Debug, Deserialize)]
pub struct GetConnectionResponse {
    pub name: String,
    pub status: String,
    pub addr: String,
}

pub async fn get_all_connections() -> Result<Vec<GetConnectionResponse>, anyhow::Error> {
    get("/_connections").await
}

// Peers:

#[derive(Debug, Deserialize)]
pub struct GetPeerResponse {
    pub addr: String,
    pub status: String,
}

pub async fn get_all_peers() -> Result<Vec<GetPeerResponse>, anyhow::Error> {
    get("/_peers").await
}

// Objects:

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

#[derive(Debug, Serialize)]
pub struct Keypair {
    pub public_key: String,
    pub private_key: String,
}

#[derive(Debug, Serialize)]
pub struct PostSeriesOwnerRequest<'a> {
    pub series_owner_name: &'a str,
    pub keypair: Option<Keypair>,
    pub is_draft: bool,
}

#[derive(Deserialize)]
pub struct PostSeriesOwnerResponse {
    pub name: String,
    pub keypair: ed25519_dalek::SigningKey,
    #[serde(with = "humantime_serde")]
    pub default_ttl: Duration,
}

type GetSeriesOwnerResponse = PostSeriesOwnerResponse;

pub async fn post_series_owner(
    request: PostSeriesOwnerRequest<'_>,
) -> Result<PostSeriesOwnerResponse, anyhow::Error> {
    post("/_series-owners", request).await
}

pub async fn delete_series_owner(series_name: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_series-owners/{series_name}")).await
}

pub async fn get_series_owner(series_name: &str) -> Result<GetSeriesOwnerResponse, anyhow::Error> {
    get(format!("/_series-owners/{series_name}")).await
}

pub async fn get_all_series_owners() -> Result<Vec<GetSeriesOwnerResponse>, anyhow::Error> {
    get("/_series-owners").await
}

// Series:

#[derive(Deserialize)]
pub struct GetSeriesResponse {
    pub public_key: Key,
}

pub async fn get_all_series() -> Result<Vec<GetSeriesResponse>, anyhow::Error> {
    get("/_series").await
}

// Editions:

#[derive(Deserialize)]
pub struct GetEditionResponse {
    pub signed: Signed<EditionContent>,
    pub public_key: Key,
    #[serde(default)]
    pub is_draft: bool,
}

pub async fn get_all_editions() -> Result<Vec<GetEditionResponse>, anyhow::Error> {
    get("/_editions").await
}

// Auth:

/// A name of an entity inside the Samizdat network. An entity can be an object, a
/// collection item, a series item, etc...
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    /// The type of the entity.
    pub r#type: String,
    /// The identifier of the entity.
    pub identifier: String,
}

#[derive(Deserialize)]
pub struct GetAuthRequest {
    pub entity: Entity,
    pub granted_rights: Vec<String>,
}

pub async fn get_auths() -> Result<Vec<GetAuthRequest>, anyhow::Error> {
    get("/_auth").await
}

#[derive(Serialize)]
pub struct PatchAuthRequest {
    pub granted_rights: Vec<String>,
}

pub async fn patch_auth(scope: &str, request: PatchAuthRequest) -> Result<bool, anyhow::Error> {
    patch(format!("/_auth/{scope}"), request).await
}

pub async fn delete_auth(scope: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_auth/{scope}")).await
}

// Collections:

#[derive(Debug, Serialize)]
pub struct PostCollectionRequest<'a> {
    pub hashes: &'a [(String, String)],
    pub is_draft: bool,
}

#[allow(unused)]
pub async fn post_collection(request: PostCollectionRequest<'_>) -> Result<String, anyhow::Error> {
    post("/_collections", request).await
}

pub async fn get_collection_list(collection: &str) -> Result<Vec<String>, anyhow::Error> {
    get(format!("/_collections/{collection}/_list")).await
}

// Subscriptions:

#[derive(Debug, Serialize)]
pub struct PostSubscriptionRequest<'a> {
    pub public_key: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct GetSubscriptionResponse {
    pub public_key: Key,
    pub kind: String,
}

pub async fn post_subscription(
    request: PostSubscriptionRequest<'_>,
) -> Result<String, anyhow::Error> {
    post("/_subscriptions", request).await
}

pub async fn get_subscription_refresh(public_key: &str) -> Result<(), anyhow::Error> {
    get(format!("/_subscriptions/{public_key}/refresh")).await
}

pub async fn delete_subscription(public_key: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_subscriptions/{public_key}")).await
}

pub async fn get_all_subscriptions() -> Result<Vec<GetSubscriptionResponse>, anyhow::Error> {
    get("/_subscriptions").await
}

// Editions:

/// The kind of an edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EditionKind {
    /// Forget everything that came before. All the content will start from scratch.
    Base,
    /// Add to what came before. If an item is not found in the current edition, search for the
    /// content in previous editions (unless _explicitely deleted_).
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

#[derive(Debug, Serialize)]
pub struct PostEditionRequest<'a> {
    pub kind: EditionKind,
    pub ttl: Option<&'a str>,
    pub no_announce: bool,
    pub is_draft: bool,
    pub hashes: &'a [(String, String)],
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionRef {
    pub hash: Hash,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct EditionContent {
    pub kind: EditionKind,
    pub collection: CollectionRef,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(with = "humantime_serde")]
    pub ttl: Duration,
}

#[derive(Debug, Deserialize)]
pub struct PostEditionResponse {
    pub signed: Signed<EditionContent>,
}

pub async fn post_edition(
    series_name: &str,
    request: PostEditionRequest<'_>,
) -> Result<PostEditionResponse, anyhow::Error> {
    post(format!("/_series-owners/{series_name}/editions",), request).await
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct SeriesRef {
    pub public_key: Key,
}

// Identities:

#[derive(Debug, Deserialize)]
pub struct PutEthereumProviderResponse {}

pub async fn put_ethereum_provider(
    endpoint: String,
) -> Result<PutEthereumProviderResponse, anyhow::Error> {
    put("/_ethereum-provider", GetEthereumProvider { endpoint }).await
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetEthereumProvider {
    pub endpoint: String,
}

pub async fn get_ethereum_provider() -> Result<GetEthereumProvider, anyhow::Error> {
    get("/_ethereum-provider").await
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

pub async fn post_vacuum() -> Result<VacuumStatus, anyhow::Error> {
    post("/_vacuum", ()).await
}

pub async fn post_flush_all() -> Result<(), anyhow::Error> {
    post("/_vacuum/flush-all", ()).await
}
