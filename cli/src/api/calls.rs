use anyhow::Context;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use samizdat_common::{pow::ProofOfWork, Hash, Key, Signed};

use super::{access_token, delete, get, patch, post, ApiError, CLIENT};

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
    pub direct_addr: String,
    pub reverse_addr: String,
}

pub async fn get_all_connections() -> Result<Vec<GetConnectionResponse>, anyhow::Error> {
    get("/_connections").await
}

// Peers:

#[derive(Debug, Deserialize)]
pub struct GetPeerResponse {
    pub hub_name: String,
    pub addr: String,
    pub is_closed: bool,
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
    let url = format!("{}/_objects", crate::server());
    let response = CLIENT
        .post(&format!(
            "{}/_objects?bookmark={}&is-draft={}",
            crate::server(),
            bookmark,
            is_draft,
        ))
        .header("Content-Type", content_type)
        .header("Authorization", format!("Bearer {}", access_token()))
        .body(content)
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST /_objects"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .with_context(|| format!("error from samizdat-node response POST /_objects"))?;

    log::info!("{} GET {} {}", status, url, text);

    let content: Result<String, ApiError> = serde_json::from_str(&text)
        .with_context(|| format!("error deserializing response from POST /_objects: {text}"))?;

    Ok(content?)
}

pub async fn get_object<F>(hash: &str, mut each_chunk: F) -> Result<(), anyhow::Error>
where
    F: FnMut(Vec<u8>) -> Result<(), anyhow::Error>,
{
    let url = format!("{}/_objects/{}", crate::server(), hash);
    let response = CLIENT
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token()))
        .send()
        .await
        .with_context(|| format!("error from samizdat-node request POST /_objects"))?;
    let status = response.status();
    log::info!("{} GET {}", status, url);

    if !status.is_success() {
        anyhow::bail!(
            "{}",
            response
                .text()
                .await
                .with_context(|| format!("error from samizdat-node response POST /_objects"))?
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
    pub keypair: ed25519_dalek::Keypair,
    #[serde(with = "humantime_serde")]
    pub default_ttl: Duration,
}

type GetSeriesOwnerResponse = PostSeriesOwnerResponse;

pub async fn post_series_owner(
    request: PostSeriesOwnerRequest<'_>,
) -> Result<PostSeriesOwnerResponse, anyhow::Error> {
    post("/_seriesowners", request).await
}

pub async fn delete_series_owner(series_name: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_seriesowners/{series_name}")).await
}

pub async fn get_series_owner(series_name: &str) -> Result<GetSeriesOwnerResponse, anyhow::Error> {
    get(format!("/_seriesowners/{series_name}")).await
}

pub async fn get_all_series_owners() -> Result<Vec<GetSeriesOwnerResponse>, anyhow::Error> {
    get("/_seriesowners").await
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
    get(format!("/_subscriptions/{public_key}/refresh")).await?;
    Ok(())
}

pub async fn delete_subscription(public_key: &str) -> Result<bool, anyhow::Error> {
    delete(format!("/_subscriptions/{public_key}")).await
}

pub async fn get_all_subscriptions() -> Result<Vec<GetSubscriptionResponse>, anyhow::Error> {
    get("/_subscriptions").await
}

// Editions:

#[derive(Debug, Serialize)]
pub struct PostEditionRequest<'a> {
    pub collection: &'a str,
    pub ttl: Option<&'a str>,
    pub no_announce: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionRef {
    pub hash: Hash,
}

#[derive(Debug, Deserialize)]
pub struct EditionContent {
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
    post(format!("/_seriesowners/{series_name}/editions",), request).await
}

#[derive(Debug, Serialize)]
pub struct PostIdentityRequest<'a> {
    pub identity: &'a str,
    pub series: &'a str,
    pub proof: ProofOfWork,
}

#[derive(Debug, Deserialize)]
pub struct IdentityRef {
    pub handle: String,
}

#[derive(Debug, Deserialize)]
pub struct SeriesRef {
    pub public_key: Key,
}

#[derive(Debug, Deserialize)]
pub struct GetIdentityResponse {
    pub identity: IdentityRef,
    pub series: SeriesRef,
    pub proof: ProofOfWork,
}

pub async fn post_identity(request: PostIdentityRequest<'_>) -> Result<bool, anyhow::Error> {
    post("/_identities", request).await
}

pub async fn get_all_identities() -> Result<Vec<GetIdentityResponse>, anyhow::Error> {
    get("/_identities").await
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
