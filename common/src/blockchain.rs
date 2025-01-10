//! Constants and configuration for blockchain interaction in the Samizdat network.
//!
//! This module defines the essential constants for interacting with the Polygon blockchain,
//! including API endpoints, contract addresses, and network parameters.

use std::time::Duration;

/// Default RPC endpoint for the Polygon network
pub const DEFAULT_PROVIDER_ENDPOINT: &str = "https://polygon.llamarpc.com";

/// Base URL for the Polygon block explorer
pub const ETHERSCAN_ENDPOINT: &str = "https://polygonscan.com";

/// API endpoint for Polygonscan
pub const ETHERSCAN_API_ENDPOINT: &str = "https://api.polygonscan.com/api";

/// API key for accessing Polygonscan services
pub const ETHERSCAN_API_KEY: &str = "W7XB8EWTZ8IS32AZVTTUN69Y6MI4XVJAS3";

/// Address of the storage smart contract on Polygon
pub const STORAGE_CONTRACT_ADDRESS: &str = "0xd0c69387e7b73c40ed712634f8738cdf28947c94";

/// Address of the manager smart contract on Polygon
pub const MANAGER_CONTRACT_ADDRESS: &str = "0x1A59C958c9d3955b594bB2145817f7f000f1F498";

/// Native token symbol for the Polygon network
pub const TOKEN_NAME: &str = "MATIC";

/// Chain ID for the Polygon network
pub const BLOCKCHAIN_ID: u64 = 137;

/// Minimum time between consecutive blockchain operations
pub const THROTTLE_LIMIT: Duration = Duration::from_millis(1_000);
