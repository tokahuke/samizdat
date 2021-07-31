use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Riddle {
    pub rand: [u8; 28],
    pub hash: [u8; 28],
}

#[tarpc::service]
pub trait Hub {
    /// Returns a greeting for name.
    async fn query(riddle: Riddle);
}

#[tarpc::service]
pub trait Node {
    async fn resolve(riddle: Riddle);
}
