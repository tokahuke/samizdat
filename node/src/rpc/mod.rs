use serde_derive::{Deserialize, Serialize};
use std::net::SocketAddr;
//use tarpc::server::{self, Channel, Incoming};
use tarpc::{context};
use tokio::net::TcpStream;

use samizdat_common::{Hash, transport};

#[derive(Debug, Serialize, Deserialize)]
struct Riddle {
    rand: [u8; 28],
    hash: [u8; 28],
}

#[tarpc::service]
trait Hub {
    /// Returns a greeting for name.
    async fn query(riddle: Riddle);
}

// #[tarpc::service]
// trait Node {
//     async fn resolve(riddle: Riddle);
// }

// struct NodeServer;

// #[tarpc::server]
// impl Node for NodeServer {
//     async fn resolve(self, _: context::Context, riddle: Riddle) {

//     }
// }

pub struct HubConnection {
    client: HubClient,
}

impl HubConnection {
    pub async fn connect(addr: impl Into<SocketAddr>) -> Result<HubConnection, crate::Error> {
        let connect = transport::Multiplex::new(TcpStream::connect(addr.into()).await.unwrap()).channel(0).await.unwrap();
        let client = HubClient::new(tarpc::client::Config::default(), connect)
            .spawn()
            .unwrap();

        Ok(HubConnection { client })
    }

    pub async fn query(&self, content_hash: Hash) -> Result<(), crate::Error>{
        let mut rand = [0; 28];
        getrandom::getrandom(&mut rand).expect("getrandom failed");
        let hash = Hash::build([rand, content_hash.0].concat());
        let riddle = Riddle { rand, hash: hash.0};
        dbg!(&riddle);
        Ok(self.client.query(context::current(), riddle).await?)
    }
}
