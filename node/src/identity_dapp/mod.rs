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

pub static IDENTITY_CACHE: LazyLock<RwLock<BTreeMap<String, Arc<Identity>>>> =
    LazyLock::new(RwLock::default);

static IDENTITY_PROVIDER: OnceLock<IdentityProvider> = OnceLock::new();

pub fn init_identity_provider() -> Result<(), crate::Error> {
    let provider = if let Some(provider) = readonly_tx(|tx| {
        Table::Global.get(tx, "ethereum_provider_endpoint", |endpoint| {
            IdentityProvider::new(String::from_utf8_lossy(endpoint).as_ref())
        })
    }) {
        provider
    } else {
        tracing::warn!(
            "Ethereum provider endpoint not set. Using default: {}",
            blockchain::DEFAULT_PROVIDER_ENDPOINT
        );
        IdentityProvider::new(blockchain::DEFAULT_PROVIDER_ENDPOINT)
    };

    IDENTITY_PROVIDER.set(provider).ok();

    Ok(())
}

pub fn identity_provider<'a>() -> &'a IdentityProvider {
    IDENTITY_PROVIDER
        .get()
        .expect("identity provider not initialized")
}

#[derive(Debug)]
pub struct Identity {
    entity: String,
    identity: String,
    ttl: u64,
    valid_until: chrono::DateTime<Utc>,
}

impl Identity {
    /// Checks if this is the "null" identity. This is the way the blockchain indicates
    /// that the identity does not exist.
    fn is_null(&self) -> bool {
        self.ttl == 0
    }

    pub fn series(&self) -> Result<SeriesRef, crate::Error> {
        self.entity.parse::<SeriesRef>()
    }
}

pub struct IdentityProvider {
    storage_contract: RwLock<Contract<Provider<Http>>>,
    // manager_contract: Contract<Provider<Http>>,
}

impl IdentityProvider {
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
