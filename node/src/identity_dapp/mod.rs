//! Provides functionality for interacting with the smart contracts that store the
//! identities for Samizdat.

use chrono::{Duration, Utc};
use ethers::abi::Abi;
use ethers::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, OnceLock},
};
use tokio::sync::RwLock;

use samizdat_common::{
    blockchain,
    db::{readonly_tx, writable_tx, Table as _},
};

use crate::{db::Table, models::SeriesRef};

/// A cache mapping identity strings to Identity instances, used to avoid redundant
/// blockchain queries.
pub static IDENTITY_CACHE: LazyLock<RwLock<BTreeMap<String, Arc<Identity>>>> =
    LazyLock::new(RwLock::default);

/// The global identity provider instance used for blockchain interactions.
static IDENTITY_PROVIDER: OnceLock<IdentityProvider> = OnceLock::new();

/// Initializes the identity provider with an endpoint from the database or uses the
/// default endpoint.
pub fn init_identity_provider() -> Result<(), crate::Error> {
    let provider = if let Some(provider) = readonly_tx(|tx| {
        Table::Global.get(tx, "ethereum_provider_endpoint", |endpoint| {
            IdentityProvider::new(String::from_utf8_lossy(endpoint).as_ref())
        })
    }) {
        provider
    } else {
        tracing::info!(
            "Ethereum provider endpoint not set. Using default: {}",
            blockchain::DEFAULT_PROVIDER_ENDPOINT
        );
        IdentityProvider::new(blockchain::DEFAULT_PROVIDER_ENDPOINT)
    };

    IDENTITY_PROVIDER.set(provider).ok();

    Ok(())
}

/// Returns a reference to the initialized identity provider.
///
/// # Panics
///
/// Panics if the identity provider has not been initialized.
pub fn identity_provider<'a>() -> &'a IdentityProvider {
    IDENTITY_PROVIDER
        .get()
        .expect("identity provider not initialized")
}

/// Represents an identity stored on the blockchain, containing entity information and
///  validity period.
#[derive(Debug)]
pub struct Identity {
    /// The entity (usually a series reference) associated with this identity
    entity: String,
    /// The unique identifier for this identity
    identity: String,
    /// Time-to-live in seconds
    ttl: u64,
    /// Timestamp when this identity information becomes invalid
    valid_until: chrono::DateTime<Utc>,
}

impl Identity {
    /// Checks if this is the "null" identity. This is the way the blockchain indicates
    /// that the identity does not exist.
    fn is_null(&self) -> bool {
        self.ttl == 0
    }

    /// Attempts to parse the entity string as a SeriesRef.
    pub fn series(&self) -> Result<SeriesRef, crate::Error> {
        self.entity.parse::<SeriesRef>()
    }
}

/// Provides functionality to interact with identity-related smart contracts on the
/// blockchain.
pub struct IdentityProvider {
    /// Contract instance for the contract that stores identities
    storage_contract: RwLock<Contract<Provider<Http>>>,
}

impl IdentityProvider {
    /// Creates a new IdentityProvider instance connected to the specified endpoint.
    pub fn new(endpoint: &str) -> IdentityProvider {
        let rpc_client = Arc::new(
            Provider::<Http>::try_from(endpoint).expect("could not instantiate HTTP Provider"),
        );
        let abi: Abi =
            serde_json::from_str(include_str!("../../../blockchain/SamizdatStorage.json"))
                .expect("SamizdatStorage abi is valid");
        let storage_contract = Contract::new(
            blockchain::STORAGE_CONTRACT_ADDRESS
                .parse::<Address>()
                .unwrap(),
            abi,
            rpc_client.clone(),
        );
        IdentityProvider {
            storage_contract: RwLock::new(storage_contract),
        }
    }

    /// Updates the provider to use a new endpoint and saves it to the database.
    pub async fn set_endpoint(&self, new_endpoint: &str) {
        let rpc_client = Arc::new(
            Provider::<Http>::try_from(new_endpoint).expect("could not instantiate HTTP Provider"),
        );
        let abi: Abi =
            serde_json::from_str(include_str!("../../../blockchain/SamizdatStorage.json"))
                .expect("SamizdatStorage abi is valid");
        let storage_contract = Contract::new(
            blockchain::STORAGE_CONTRACT_ADDRESS
                .parse::<Address>()
                .unwrap(),
            abi,
            rpc_client.clone(),
        );

        writable_tx(|tx| {
            Table::Global.put(tx, "ethereum_provider_endpoint", new_endpoint);
            Ok(())
        })
        .expect("infalible");

        *self.storage_contract.write().await = storage_contract;
    }

    /// Retrieves identity information from the blockchain.
    pub async fn get(&self, identity: &str) -> Result<Identity, crate::Error> {
        let (entity, _owner, ttl, _data) = self
            .storage_contract
            .read()
            .await
            .method::<_, (String, Address, u64, Vec<u8>)>("identities", identity.to_owned())
            .expect("ABI was not declared as expected")
            .call()
            .await
            .map_err(|e| format!("Smart contract error: {e}"))?;
        Ok(Identity {
            entity,
            identity: identity.to_owned(),
            valid_until: Utc::now() + Duration::seconds(ttl as i64),
            ttl,
        })
    }

    /// Retrieves identity information, using a cache to avoid redundant blockchain queries.
    /// Returns `None` if the identity doesn't exist.
    pub async fn get_cached(&self, identity: &str) -> Result<Option<Arc<Identity>>, crate::Error> {
        if let Some(identity) = IDENTITY_CACHE.read().await.get(identity) {
            tracing::debug!("Found cached identity");
            if identity.valid_until > Utc::now() {
                return Ok(Some(identity.clone()));
            } else {
                tracing::debug!("Cached identity is outdated. Will have to ask the Network again")
            }
        }

        // Ok, this impl might lead to TOCTOU, but that is not an issue...
        let identity = Arc::new(self.get(identity).await?);

        // Check if it is null (inexistent) identity:
        if identity.is_null() {
            return Ok(None);
        }

        // Ok, this impl might lead to TOCTOU, but that is not an issue...
        IDENTITY_CACHE
            .write()
            .await
            .insert(identity.identity.clone(), identity.clone());

        Ok(Some(identity))
    }
}
