//! QUIC-based RPC transports using bincode. A custom codec wraps
//! `bincode` for `tarpc`, and helpers build a transport with a
//! configurable per-message size cap.

use futures::future;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tarpc::tokio_serde::{Deserializer, Serializer};

/// Bincode codec for `tarpc` transports.
struct BincodeCodec;

impl<T: Serialize> Serializer<T> for BincodeCodec {
    type Error = crate::Error;
    fn serialize(
        self: Pin<&mut Self>,
        item: &T,
    ) -> Result<tarpc::tokio_util::bytes::Bytes, Self::Error> {
        Ok(bincode::serialize(item)?.into())
    }
}

impl<T: for<'a> Deserialize<'a>> Deserializer<T> for BincodeCodec {
    type Error = crate::Error;
    fn deserialize(
        self: Pin<&mut Self>,
        src: &tarpc::tokio_util::bytes::BytesMut,
    ) -> Result<T, Self::Error> {
        Ok(bincode::deserialize(src)?)
    }
}

/// Build a bincode transport over a pair of QUIC streams, capping each
/// message at `max_size` bytes.
pub fn bincode_transport<S, R>(
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    max_size: usize,
) -> impl tarpc::Transport<S, R>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    let mut limiter = tarpc::tokio_util::codec::LengthDelimitedCodec::new();
    limiter.set_max_frame_length(max_size);
    tarpc::serde_transport::new(
        tarpc::tokio_util::codec::Framed::new(tokio::io::join(recv, send), limiter),
        BincodeCodec,
    )
}

/// Open a new bidirectional bincode transport on `connection`.
pub async fn open_bincode_transport<S, R>(
    connection: quinn::Connection,
    max_size: usize,
) -> Result<impl tarpc::Transport<S, R>, crate::Error>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    let (send, recv) = connection.open_bi().await?;
    Ok(bincode_transport(send, recv, max_size))
}

/// Accept a new bidirectional bincode transport from `connection`.
pub async fn accept_bincode_transport<S, R>(
    connection: quinn::Connection,
    max_size: usize,
) -> Result<impl tarpc::Transport<S, R>, crate::Error>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    let (send, recv) = connection.accept_bi().await?;
    Ok(bincode_transport(send, recv, max_size))
}

/// Length of the hello byte in a channel handshake.
const HELLO_SIZE: usize = 1;

/// Hello byte for a direct channel.
const DIRECT_CHANNEL_HELLO: [u8; HELLO_SIZE] = *b"d";

/// Hello byte for a reverse channel.
const REVERSE_CHANNEL_HELLO: [u8; HELLO_SIZE] = *b"r";

/// Open a direct-channel bincode transport (sends the `d` hello first).
pub async fn open_direct_bincode_transport<S, R>(
    connection: quinn::Connection,
    max_size: usize,
) -> Result<impl tarpc::Transport<S, R>, crate::Error>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    let (mut send, recv) = connection.open_bi().await?;

    send.write_all(&DIRECT_CHANNEL_HELLO)
        .await
        .map_err(|err| err.to_string())?;

    Ok(bincode_transport(send, recv, max_size))
}

/// Open a reverse-channel bincode transport (sends the `r` hello first).
pub async fn open_reverse_bincode_transport<S, R>(
    connection: quinn::Connection,
    max_size: usize,
) -> Result<impl tarpc::Transport<S, R>, crate::Error>
where
    S: 'static + Send + Serialize,
    R: 'static + Send + for<'a> Deserialize<'a>,
{
    let (mut send, recv) = connection.open_bi().await?;

    send.write_all(&REVERSE_CHANNEL_HELLO)
        .await
        .map_err(|err| err.to_string())?;

    Ok(bincode_transport(send, recv, max_size))
}

/// Accept one direct + one reverse channel from `connection`, in either
/// hello order. Returns them as `(direct, reverse)`.
pub async fn accept_bincode_transports<S1, R1, S2, R2>(
    connection: quinn::Connection,
    max_size: usize,
) -> Result<(impl tarpc::Transport<S1, R1>, impl tarpc::Transport<S2, R2>), crate::Error>
where
    S1: 'static + Send + Serialize,
    R1: 'static + Send + for<'a> Deserialize<'a>,
    S2: 'static + Send + Serialize,
    R2: 'static + Send + for<'a> Deserialize<'a>,
{
    let (send1, mut recv1) = connection.accept_bi().await?;
    let (send2, mut recv2) = connection.accept_bi().await?;

    async fn read_hello(recv: &mut quinn::RecvStream) -> Result<[u8; HELLO_SIZE], crate::Error> {
        let mut buffer = [0; HELLO_SIZE];
        recv.read_exact(&mut buffer)
            .await
            .map_err(|err| err.to_string())?;
        Ok(buffer)
    }

    let (hello1, hello2) = future::join(read_hello(&mut recv1), read_hello(&mut recv2)).await;

    match (hello1?, hello2?) {
        (DIRECT_CHANNEL_HELLO, REVERSE_CHANNEL_HELLO) => Ok((
            bincode_transport(send1, recv1, max_size),
            bincode_transport(send2, recv2, max_size),
        )),
        (REVERSE_CHANNEL_HELLO, DIRECT_CHANNEL_HELLO) => Ok((
            bincode_transport(send2, recv2, max_size),
            bincode_transport(send1, recv1, max_size),
        )),
        (bad_bytes1, bad_bytes2) => {
            let bad_hello1 = String::from_utf8_lossy(&bad_bytes1);
            let bad_hello2 = String::from_utf8_lossy(&bad_bytes2);
            Err(
                format!("received anomalous hellos: hello1={bad_hello1} hello2={bad_hello2}")
                    .into(),
            )
        }
    }
}
