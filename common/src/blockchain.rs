use std::time::Duration;

pub const DEFAULT_PROVIDER_ENDPOINT: &str = "https://eth-sepolia.public.blastapi.io";
pub const ETHERSCAN_ENDPOINT: &str = "https://sepolia.etherscan.io";
pub const ETHERSCAN_API_ENDPOINT: &str = "https://api-sepolia.etherscan.io/api";
pub const STORAGE_CONTRACT_ADDRESS: &str = "0x99A3E5472B2f83555CDd0d0E674D84C6aEE88E53";
pub const MANAGER_CONTRACT_ADDRESS: &str = "0x318b3fDdEb00Ba9193188B4e47Bf860EAE8D6F6D";
pub const BLOCKCHAIN_ID: u64 = 11155111u64; // Sepolia
pub const THROTTLE_LIMIT: Duration = Duration::from_millis(1_000);
