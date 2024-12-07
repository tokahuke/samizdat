use std::io::stdin;

use futures::SinkExt;
use futures::StreamExt;
pub use samizdat_common::Error;

use samizdat_common::quic;
use samizdat_common::transport;
use serde_derive::{Deserialize, Serialize};

#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    #[derive(Debug, Serialize, Deserialize)]
    struct Message {
        content: String,
    }

    let server = tokio::spawn(async {
        let server = quic::new_default("[::1]:45100".parse().unwrap());
        let conn = server.accept().await.unwrap().await.unwrap();
        let mut transport = transport::accept_bincode_transport::<Message, Message>(conn, 2048)
            .await
            .unwrap();
        loop {
            let msg: Message = transport.next().await.unwrap().unwrap();

            println!("client said: {}", msg.content);
        }
    });

    let local = tokio::task::LocalSet::new();

    local
        .run_until(async {
            let client = quic::new_default("[::1]:45101".parse().unwrap());
            let conn = quic::connect(&client, "[::1]:45100".parse().unwrap(), true)
                .await
                .unwrap();
            let mut transport = transport::open_bincode_transport::<Message, Message>(conn, 2048)
                .await
                .unwrap();

            loop {
                for line in stdin().lines() {
                    transport
                        .send(Message {
                            content: line.unwrap(),
                        })
                        .await
                        .unwrap();
                }
            }
        })
        .await;

    server.await.unwrap();

    Ok(())
}
