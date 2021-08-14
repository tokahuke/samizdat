//! TODO also secure **headers** using the focus hash in each case.

mod cipher;

use futures::prelude::*;
use futures::stream;
use futures::TryStreamExt;
use quinn::{Connection, IncomingUniStreams, ReadToEndError};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_derive::{Deserialize, Serialize};
use std::io;
use std::sync::Arc;

use samizdat_common::Hash;
use samizdat_common::PatriciaProof;

use crate::cache::{CollectionItem, Locator, ObjectRef};
use crate::cli;

use self::cipher::TransferCipher;

const MAX_HEADER_LENGTH: usize = 4_096;
const MAX_STREAM_SIZE: usize = crate::cache::CHUNK_SIZE * 2;

fn read_error_to_io(error: ReadToEndError) -> io::Error {
    match error {
        ReadToEndError::TooLong => io::Error::new(io::ErrorKind::InvalidData, "too long"),
        ReadToEndError::Read(read) => io::Error::from(read),
    }
}

#[async_trait::async_trait]
trait Header: 'static + Send + Sync + SerdeSerialize + for<'a> SerdeDeserialize<'a> {
    async fn recv(uni_streams: &mut IncomingUniStreams, cipher: &TransferCipher) -> Result<Self, crate::Error> {
        // Receive header from peer:
        let header_stream = uni_streams
            .next()
            .await
            .ok_or_else(|| "connection dried!".to_owned())??;
        let mut serialized_header = header_stream
            .read_to_end(MAX_HEADER_LENGTH)
            .await
            .map_err(read_error_to_io)?;
        cipher.decrypt(&mut serialized_header);
        let header: Self = bincode::deserialize(&serialized_header)?;

        Ok(header)
    }

    async fn send(&self, connection: &Connection, cipher: &TransferCipher) -> Result<(), crate::Error> {
        let mut send_header = connection.open_uni().await?;
        log::debug!("stream for header opened");

        let mut serialized_header = bincode::serialize(&self).expect("can serialize");
        cipher.encrypt(&mut serialized_header);
        send_header
            .write_all(&serialized_header)
            .await
            .map_err(io::Error::from)?;
        log::debug!("header streamed");
        send_header.finish().await.map_err(io::Error::from)?;
        log::debug!("header sent");

        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
struct NonceHeader {
    nonce: Hash,
}

impl Header for NonceHeader {}

impl NonceHeader {
    fn new() -> NonceHeader {
        NonceHeader {
            nonce: Hash::rand(),
        }
    }

    fn cipher(self, hash: Hash) -> TransferCipher {
        TransferCipher::new(&hash, &self.nonce)
    }

    async fn recv_negotiate(uni_streams: &mut IncomingUniStreams, hash: Hash) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceHeader::default().cipher(hash);
        let nonce_header = NonceHeader::recv(uni_streams, &init_cipher).await?;
        
        Ok(nonce_header.cipher(hash))
    }

    async fn send_negotiate(connection: &Connection, hash: Hash) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceHeader::default().cipher(hash);
        let nonce_header = NonceHeader::new();
        nonce_header.send(connection, &init_cipher).await?;
    
        Ok(nonce_header.cipher(hash))
    }
}

#[derive(Serialize, Deserialize)]
struct ItemHeader {
    inclusion_proof: PatriciaProof,
    object_header: ObjectHeader,
}

impl Header for ItemHeader {}

impl ItemHeader {
    fn for_item(item: CollectionItem) -> Result<ItemHeader, crate::Error> {
        let object_header = ObjectHeader::for_object(&item.object()?)?;
        Ok(ItemHeader {
            inclusion_proof: item.inclusion_proof,
            object_header,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ObjectHeader {
    nonce: Hash,
    content_size: usize,
    content_type: String,
}

impl Header for ObjectHeader {}

impl ObjectHeader {
    /// # Panics:
    /// If object does not exist locally.
    fn for_object(object: &ObjectRef) -> Result<ObjectHeader, crate::Error> {
        let metadata = object.metadata()?.expect("object exists");

        Ok(ObjectHeader {
            nonce: Hash::rand(),
            content_size: metadata.content_size,
            content_type: metadata.content_type,
        })
    }

    pub async fn recv_data(
        self,
        uni_streams: &mut IncomingUniStreams,
        hash: Hash,
    ) -> Result<ObjectRef, crate::Error> {
        let cipher = Arc::new(TransferCipher::new(&hash, &self.nonce));

        // Refuse if content is too big:
        if self.content_size > cli().max_content_size {
            return Err(format!(
                "content too big: max size is {}, advertised was {}",
                cli().max_content_size,
                self.content_size
            )
            .into());
        }

        // Stream the content:
        let content_stream = uni_streams
            .map_err(io::Error::from)
            .and_then(|stream| {
                let cipher = cipher.clone();
                async move {
                    log::debug!("receiving chunk");
                    stream
                        .read_to_end(MAX_STREAM_SIZE)
                        .await
                        .map_err(read_error_to_io)
                        .map(|mut buffer| {
                            cipher.decrypt(&mut buffer);
                            stream::iter(
                                buffer
                                    .into_iter()
                                    .map(|byte| Ok(byte) as Result<_, io::Error>),
                            )
                        })
                }
            })
            .try_flatten()
            .map_err(crate::Error::from);

        // Build content from stream (this limits content size to the advertised amount)
        let (metadata, object) = ObjectRef::build(
            self.content_type,
            self.content_size,
            Box::pin(content_stream),
        )
        .await?;

        // Check if the peer is up to any extra sneaky tricks.
        if metadata.content_size != self.content_size {
            Err(format!(
                "actual data length did not match content-size: expected {}, got {}",
                metadata.content_size, self.content_size
            )
            .into())
        } else if object.hash != hash {
            Err(format!(
                "bad content from peer: expected {}, got {}",
                object.hash, hash
            )
            .into())
        } else {
            Ok(object)
        }
    }

    pub async fn send_data(
        self,
        connection: &Connection,
        object: &ObjectRef,
    ) -> Result<(), crate::Error> {
        let cipher = TransferCipher::new(&object.hash, &self.nonce);

        for chunk in object.iter()?.expect("object exits") {
            let mut chunk = chunk?;
            let mut send_data = connection.open_uni().await?;
            log::debug!("stream for data opened");
            cipher.encrypt(&mut chunk);
            send_data.write_all(&chunk).await.map_err(io::Error::from)?;
            log::debug!("data streamed");
            send_data.finish().await.map_err(io::Error::from)?;
            log::debug!("data sent");
        }

        log::info!(
            "finished sending {} to {}",
            object.hash,
            connection.remote_address()
        );

        Ok(())
    }
}

pub async fn recv_object(
    uni_streams: &mut IncomingUniStreams,
    hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    let transfer_cipher = NonceHeader::recv_negotiate(uni_streams, hash).await?;
    let header = ObjectHeader::recv(uni_streams, &transfer_cipher).await?;
    header.recv_data(uni_streams, hash).await
}

pub async fn send_object(connection: &Connection, object: &ObjectRef) -> Result<(), crate::Error> {
    let transfer_cipher = NonceHeader::send_negotiate(connection, object.hash).await?;
    let header = ObjectHeader::for_object(object)?;
    header.send(connection, &transfer_cipher).await?;
    header.send_data(connection, object).await
}

pub async fn recv_item(
    uni_streams: &mut IncomingUniStreams,
    locator: Locator<'_>,
) -> Result<ObjectRef, crate::Error> {
    let hash = locator.hash();
    let transfer_cipher = NonceHeader::recv_negotiate(uni_streams, hash).await?;
    let header = ItemHeader::recv(uni_streams, &transfer_cipher).await?;
    header
        .object_header
        .recv_data(uni_streams, hash)
        .await
}

pub async fn send_item(connection: &Connection, item: CollectionItem) -> Result<(), crate::Error> {
    let object = item.object()?;
    let hash = item.locator().hash();
    let header = ItemHeader::for_item(item)?;
    let transfer_cipher = NonceHeader::send_negotiate(connection, hash).await?;
    header.send(connection, &transfer_cipher).await?;
    header.object_header.send_data(connection, &object).await
}
