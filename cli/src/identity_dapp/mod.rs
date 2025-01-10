//! Identity management module for the Samizdat CLI.

use std::{io::Write, sync::Arc};

use anyhow::Context;
use ethers::{abi::Abi, etherscan, prelude::*};

use samizdat_common::blockchain;

use crate::util::MARKER;

/// Introduces a delay between blockchain operations to prevent provider throttling.
async fn wait() {
    tokio::time::sleep(blockchain::THROTTLE_LIMIT).await;
}

/// Prompts the user for input with a given prompt string.
/// 
/// # Returns
/// The user's input as a String with trailing whitespace removed.
fn read(prompt: &str) -> String {
    print!("{MARKER} {prompt}: ");
    std::io::stdout().lock().flush().expect("Failed to flush");
    let mut response = String::new();
    std::io::stdin()
        .read_line(&mut response)
        .expect("Failed to read line");
    println!();

    response.trim_end().to_owned()
}

/// Prompts for and validates a wallet private key.
/// 
/// # Returns
/// A configured LocalWallet instance for the Polygon network.
/// 
/// # Errors
/// Returns an error if the private key is invalid or cannot be parsed.
fn get_wallet() -> Result<LocalWallet, anyhow::Error> {
    let wallet = rpassword::prompt_password(format!("{MARKER} Insert private key: "))?
        .parse::<LocalWallet>()
        .context("Bad polygon private key")?
        .with_chain_id(blockchain::BLOCKCHAIN_ID);
    println!();

    Ok(wallet)
}

/// Creates a configured Etherscan client instance.
/// 
/// # Returns
/// A configured etherscan::Client ready for blockchain interactions.
fn get_etherscan() -> etherscan::Client {
    etherscan::Client::builder()
        .with_url(blockchain::ETHERSCAN_ENDPOINT)
        .expect("Invalid etherscan URL")
        .with_api_url(blockchain::ETHERSCAN_API_ENDPOINT)
        .expect("Invalid etherscan API URL")
        .with_api_key(blockchain::ETHERSCAN_API_KEY)
        .build()
        .expect("Failed to build etherscan client")
}

/// Creates a Provider instance for interacting with the blockchain.
/// 
/// # Arguments
/// * `endpoint` - Optional RPC endpoint URL. If None, uses the default Polygon provider.
async fn client(endpoint: Option<String>) -> Result<Provider<Http>, anyhow::Error> {
    let endpoint = if let Some(url) = endpoint {
        url
    } else {
        crate::api::get_polygon_provider().await?.endpoint
    };
    Ok(Provider::<Http>::try_from(endpoint).expect("could not instantiate HTTP Provider"))
}

/// Creates a SignerMiddleware instance for authenticated blockchain interactions.
/// 
/// # Arguments
/// * `endpoint` - Optional RPC endpoint URL
/// * `wallet` - The LocalWallet instance for transaction signing
async fn signing_client(
    endpoint: Option<String>,
    wallet: LocalWallet,
) -> Result<SignerMiddleware<Provider<Http>, LocalWallet>, anyhow::Error> {
    Ok(SignerMiddleware::new(client(endpoint).await?, wallet))
}

/// Creates a Contract instance for the Samizdat Identity Manager.
/// 
/// # Arguments
/// * `rpc_client` - The SignerMiddleware instance for authenticated interactions
async fn get_manager_contract(
    rpc_client: Arc<SignerMiddleware<Provider<Http>, LocalWallet>>,
) -> Result<Contract<SignerMiddleware<Provider<Http>, LocalWallet>>, anyhow::Error> {
    let abi: Abi =
        serde_json::from_str(include_str!("../../../blockchain/SamizdatIdentityV1.json"))
            .expect("SamizdatStorage abi is valid");
    Ok(Contract::new(
        blockchain::MANAGER_CONTRACT_ADDRESS
            .parse::<Address>()
            .unwrap(),
        abi,
        rpc_client,
    ))
}

/// Creates a Contract instance for the Samizdat Storage contract.
/// 
/// # Arguments
/// * `endpoint` - Optional RPC endpoint URL
async fn get_storage_contract(
    endpoint: Option<String>,
) -> Result<Contract<Provider<Http>>, anyhow::Error> {
    let endpoint = if let Some(url) = endpoint {
        url
    } else {
        crate::api::get_polygon_provider().await?.endpoint
    };
    let rpc_client = Arc::new(
        Provider::<Http>::try_from(endpoint).expect("could not instantiate HTTP Provider"),
    );

    let abi: Abi = serde_json::from_str(include_str!("../../../blockchain/SamizdatStorage.json"))
        .expect("SamizdatStorage abi is valid");
    Ok(Contract::new(
        blockchain::STORAGE_CONTRACT_ADDRESS
            .parse::<Address>()
            .unwrap(),
        abi,
        rpc_client,
    ))
}

/// Creates a new identity registration on the blockchain.
/// 
/// # Arguments
/// * `identity` - The identity string to register
/// * `entity` - The entity string to associate with the identity
/// * `ttl` - Time-to-live value for the registration
/// * `endpoint` - Optional RPC endpoint URL
/// 
/// # Errors
/// Returns an error if the transaction fails or is rejected by the contract.
pub async fn create(
    identity: String,
    entity: String,
    ttl: u64,
    endpoint: Option<String>,
) -> Result<(), anyhow::Error> {
    let wallet = get_wallet()?;
    let etherscan = get_etherscan();
    let rpc_client = Arc::new(signing_client(endpoint, wallet.clone()).await?);
    let manager_contract = get_manager_contract(rpc_client.clone()).await?;
    let price_in_wei = manager_contract
        .method::<_, u64>("price", ())
        .expect("ABI was not declared as expected")
        .call()
        .await
        .context("Smart contract error")?;

    let mut register = manager_contract
        .method::<_, ()>("registerWithTtl", (identity.clone(), entity.clone(), ttl))
        .expect("ABI was not declared as expected")
        .value(price_in_wei)
        .from(wallet.address());

    // Gas shenanigans:
    wait().await;
    let gas_estimate = match register.estimate_gas().await {
        Ok(estimate) => estimate,
        Err(err) => {
            let Some(revert): Option<String> = err.decode_revert() else {
                Err(err).context("Estimating gas for `register` transaction")?
            };
            anyhow::bail!("Contract says: {revert}");
        }
    };
    let gas_price = etherscan.gas_oracle().await?.propose_gas_price;
    register = register.gas(gas_estimate);
    register = register.gas_price(gas_price);

    println!("{MARKER} Claiming {identity:?} as {entity:?} with TTL of {ttl}");
    println!("  Using funds from: {}", wallet.address());
    println!(
        "  Price to register: {}{}",
        price_in_wei as f64 / 1_000_000_000_000_000_000f64,
        blockchain::TOKEN_NAME
    );
    println!("  Gas estimate: {gas_estimate:?}");
    println!("  Gas price: {}Gwei", gas_price.as_u64() as f64 / 1e9);
    println!();
    let response = read("Type \"yes\" to proceed");
    if response.trim_end() != "yes" {
        println!("Not proceeding with transaction.");
        return Ok(());
    }

    wait().await;
    let pending_tx = register
        .send()
        .await
        .context("Sending `registerWithTtl` transaction to Polygon")?;
    let tx_hash = pending_tx.tx_hash();
    pending_tx
        .await
        .context("Waiting for confirmation of the `registerWithTtl` transaction")?;

    println!(
        "{MARKER} View transaction at {}",
        etherscan.transaction_url(tx_hash)
    );

    Ok(())
}

/// Updates an existing identity registration on the blockchain.
/// 
/// # Arguments
/// * `identity` - The identity string to update
/// * `entity` - The new entity string to associate with the identity
/// * `ttl` - New time-to-live value for the registration
/// * `endpoint` - Optional RPC endpoint URL
/// 
/// # Errors
/// Returns an error if the transaction fails or is rejected by the contract.
pub async fn update(
    identity: String,
    entity: String,
    ttl: u64,
    endpoint: Option<String>,
) -> Result<(), anyhow::Error> {
    let wallet = get_wallet()?;
    let etherscan = get_etherscan();
    let rpc_client = Arc::new(signing_client(endpoint, wallet.clone()).await?);
    let manager_contract = get_manager_contract(rpc_client.clone()).await?;
    let mut register = manager_contract
        .method::<_, ()>("registerWithTtl", (identity.clone(), entity.clone(), ttl))
        .expect("ABI was not declared as expected")
        .value(0)
        .from(wallet.address());

    wait().await;
    let gas_estimate = match register.estimate_gas().await {
        Ok(estimate) => estimate,
        Err(err) => {
            let Some(revert): Option<String> = err.decode_revert() else {
                Err(err).context("Estimating gas for `register` transaction")?
            };
            anyhow::bail!("Contract says: {revert}");
        }
    };
    register = register.gas(gas_estimate);

    println!("{MARKER} Claiming {identity:?} as {entity:?} with TTL of {ttl}");
    println!("  Using funds from: {}", wallet.address());
    println!("  Price to update: <not charged>",);
    println!("  Gas estimate: {gas_estimate:?}");
    println!();
    let response = read("Type \"yes\" to proceed");
    if response.trim_end() != "yes" {
        println!("Not proceeding with transaction.");
        return Ok(());
    }

    wait().await;
    let pending_tx = register
        .send()
        .await
        .context("Sending `register` transaction to Polygon")?;
    let tx_hash = pending_tx.tx_hash();
    pending_tx
        .await
        .context("Waiting for confirmation of the `register` transaction")?;

    println!(
        "{MARKER} View transaction at {}",
        etherscan.transaction_url(tx_hash)
    );

    Ok(())
}

/// Retrieves the entity associated with an identity from the blockchain.
/// 
/// # Arguments
/// * `identity` - The identity string to look up
/// * `endpoint` - Optional RPC endpoint URL
/// 
/// # Returns
/// The entity string associated with the identity.
/// 
/// # Errors
/// Returns an error if the identity doesn't exist or the contract call fails.
pub async fn get(identity: String, endpoint: Option<String>) -> Result<String, anyhow::Error> {
    let storage_contract = get_storage_contract(endpoint).await?;
    let (entity, _owner, _ttl, _data) = storage_contract
        .method::<_, (String, Address, u64, Vec<u8>)>("identities", identity.to_owned())
        .expect("ABI was not declared as expected")
        .call()
        .await
        .context("Smart contract error")?;
    Ok(entity)
}
