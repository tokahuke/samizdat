use std::{io::Write, sync::Arc};

use anyhow::Context;
use ethers::{abi::Abi, etherscan, prelude::*};

use samizdat_common::blockchain;

use crate::util::MARKER;

// A waiting period to make the provider not throttle us.
async fn wait() {
    tokio::time::sleep(blockchain::THROTTLE_LIMIT).await;
}

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

fn get_wallet() -> Result<LocalWallet, anyhow::Error> {
    let wallet = rpassword::prompt_password(format!("{MARKER} Insert private key: "))?
        .parse::<LocalWallet>()
        .context("Bad ETH private key")?
        .with_chain_id(blockchain::BLOCKCHAIN_ID);
    println!();

    Ok(wallet)
}

fn get_etherscan() -> etherscan::Client {
    etherscan::Client::builder()
        .with_url(blockchain::ETHERSCAN_ENDPOINT)
        .expect("Invalid etherscan URL")
        .with_api_url(blockchain::ETHERSCAN_API_ENDPOINT)
        .expect("Invalid etherscan API URL")
        .build()
        .expect("Failed to build etherscan client")
}

async fn get_manager_contract(
    wallet: LocalWallet,
) -> Result<Contract<SignerMiddleware<Provider<Http>, LocalWallet>>, anyhow::Error> {
    let endpoint = crate::api::get_ethereum_provider().await?.endpoint;
    let rpc_client = Arc::new(SignerMiddleware::new(
        Provider::<Http>::try_from(endpoint).expect("could not instantiate HTTP Provider"),
        wallet.clone(),
    ));

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

async fn get_storage_contract() -> Result<Contract<Provider<Http>>, anyhow::Error> {
    let endpoint = crate::api::get_ethereum_provider().await?.endpoint;
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

pub async fn create(identity: String, entity: String, ttl: u64) -> Result<(), anyhow::Error> {
    let wallet = get_wallet()?;
    let etherscan = get_etherscan();
    let manager_contract = get_manager_contract(wallet.clone()).await?;
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
    // .context("Estimating gas for `register` transaction")?;
    register = register.gas(gas_estimate);

    println!("{MARKER} Claiming {identity:?} as {entity:?} with TTL of {ttl}");
    println!("  Using funds from: {}", wallet.address());
    println!(
        "  Price to register: {}ETH",
        price_in_wei as f64 / 1_000_000_000_000_000_000f64
    );
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
        .context("Sending `registerWithTtl` transaction to Ethereum")?;
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

pub async fn update(identity: String, entity: String, ttl: u64) -> Result<(), anyhow::Error> {
    let wallet = get_wallet()?;
    let etherscan = get_etherscan();
    let manager_contract = get_manager_contract(wallet.clone()).await?;
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
        .context("Sending `register` transaction to Ethereum")?;
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

pub async fn get(identity: String) -> Result<String, anyhow::Error> {
    let storage_contract = get_storage_contract().await?;
    let (entity, _owner, _ttl, _data) = storage_contract
        .method::<_, (String, Address, u64, Vec<u8>)>("identities", identity.to_owned())
        .expect("ABI was not declared as expected")
        .call()
        .await
        .context("Smart contract error")?;
    Ok(entity)
}