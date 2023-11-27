use std::time::Duration;

pub const DEFAULT_PROVIDER_ENDPOINT: &str = "https://polygon.llamarpc.com";
pub const ETHERSCAN_ENDPOINT: &str = "https://polygonscan.com";
pub const ETHERSCAN_API_ENDPOINT: &str = "https://api.polygonscan.com/api";
pub const ETHERSCAN_API_KEY: &str = "W7XB8EWTZ8IS32AZVTTUN69Y6MI4XVJAS3";
pub const STORAGE_CONTRACT_ADDRESS: &str = "0xd0c69387e7b73c40ed712634f8738cdf28947c94";
pub const MANAGER_CONTRACT_ADDRESS: &str = "0x1A59C958c9d3955b594bB2145817f7f000f1F498";
pub const TOKEN_NAME: &str = "MATIC";
pub const BLOCKCHAIN_ID: u64 = 137; // Polygon
pub const THROTTLE_LIMIT: Duration = Duration::from_millis(1_000);
