use brotli::{CompressorReader, Decompressor};
use futures::prelude::*;
use futures::stream;
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_derive::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::sync::Arc;

use samizdat_common::cipher::TransferCipher;
use samizdat_common::Hash;

use crate::cli;
use crate::models::{CollectionItem, ObjectRef};

use super::transport::{ChannelReceiver, ChannelSender};

const MAX_HEADER_LENGTH: usize = 4_096;
const MAX_STREAM_SIZE: usize = crate::models::CHUNK_SIZE * 2;

#[async_trait::async_trait]
trait Header: 'static + Send + Sync + SerdeSerialize + for<'a> SerdeDeserialize<'a> {
    async fn recv(
        receiver: &mut ChannelReceiver,
        cipher: &TransferCipher,
    ) -> Result<Self, crate::Error> {
        // Receive header from peer:
        let mut serialized_header = receiver
            .recv(MAX_HEADER_LENGTH)
            .await?
            .ok_or_else(|| format!("channel dried"))?;
        cipher.decrypt(&mut serialized_header);
        let header: Self = bincode::deserialize(&serialized_header)?;

        Ok(header)
    }

    async fn send(
        &self,
        sender: &ChannelSender,
        cipher: &TransferCipher,
    ) -> Result<(), crate::Error> {
        let mut serialized_header = bincode::serialize(&self).expect("can serialize");
        cipher.encrypt(&mut serialized_header);
        sender.send(&serialized_header).await?;

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

    async fn recv_negotiate(
        receiver: &mut ChannelReceiver,
        hash: Hash,
    ) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceHeader::default().cipher(hash);
        let nonce_header = NonceHeader::recv(receiver, &init_cipher).await?;

        Ok(nonce_header.cipher(hash))
    }

    async fn send_negotiate(
        sender: &ChannelSender,
        hash: Hash,
    ) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceHeader::default().cipher(hash);
        let nonce_header = NonceHeader::new();
        nonce_header.send(sender, &init_cipher).await?;

        Ok(nonce_header.cipher(hash))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ItemHeader {
    item: CollectionItem,
    object_header: ObjectHeader,
}

impl Header for ItemHeader {}

impl ItemHeader {
    fn for_item(item: CollectionItem) -> Result<ItemHeader, crate::Error> {
        let object_header = ObjectHeader::for_object(&item.object()?)?;
        Ok(ItemHeader {
            item,
            object_header,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
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
        receiver: &mut ChannelReceiver,
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

        // Stream content
        let content_stream = receiver
            .recv_many(MAX_STREAM_SIZE)
            .map_ok(|mut buffer| {
                cipher.decrypt(&mut buffer);
                let reader = Decompressor::new(Cursor::new(buffer), 4096);
                stream::iter(
                    reader
                        .bytes()
                        .map(|byte| Ok(byte.expect("never error")) as Result<_, crate::Error>),
                )
            })
            .try_flatten();

        // Build content from stream (this limits content size to the advertised amount)
        let (metadata, object) = ObjectRef::build(
            self.content_type,
            self.content_size,
            Box::pin(content_stream),
        )
        .await?;

        log::info!("done building object");

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
                hash, object.hash,
            )
            .into())
        } else {
            log::info!("received valid object from peer");
            Ok(object)
        }
    }

    pub async fn send_data(
        self,
        sender: &ChannelSender,
        object: &ObjectRef,
    ) -> Result<(), crate::Error> {
        let cipher = TransferCipher::new(&object.hash, &self.nonce);

        for chunk in object.iter()?.expect("object exits") {
            let chunk = chunk?;
            log::info!("stream for data opened");
            let mut compressed = CompressorReader::new(Cursor::new(chunk), 4096, 4, 22)
                .bytes()
                .collect::<Result<Vec<_>, _>>()
                .expect("never error");
            cipher.encrypt(&mut compressed);
            sender.send(&compressed).await?;
        }

        log::info!(
            "finished sending {} to {}",
            object.hash,
            sender.remote_address()
        );

        Ok(())
    }
}

pub async fn recv_object(
    mut receiver: ChannelReceiver,
    hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceHeader::recv_negotiate(&mut receiver, hash).await?;
    log::info!("receiving object header");
    let header = ObjectHeader::recv(&mut receiver, &transfer_cipher).await?;
    log::info!("receiving data");
    let object = header.recv_data(&mut receiver, hash).await?;

    log::info!("done receiving object");

    Ok(object)
}

pub async fn send_object(sender: &ChannelSender, object: &ObjectRef) -> Result<(), crate::Error> {
    object.touch()?;

    let header = ObjectHeader::for_object(object)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceHeader::send_negotiate(sender, object.hash).await?;
    log::info!("sending object header");
    header.send(sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.send_data(sender, object).await?;

    log::info!("done sending object");

    Ok(())
}

/// TODO: make object transfer optional if the receiver perceives that it
/// already has the object (one simple table lookup, no seqscan here). This is
/// important as people update their collections often, but keep most of it
/// intact.
pub async fn recv_item(
    mut receiver: ChannelReceiver,
    locator_hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceHeader::recv_negotiate(&mut receiver, locator_hash).await?;
    log::info!("receiving item header");
    let header = ItemHeader::recv(&mut receiver, &transfer_cipher).await?;

    // No tricks!
    let locator_hash_from_peer = header.item.locator().hash();
    if locator_hash_from_peer != locator_hash {
        return Err(crate::Error::Message(format!(
            "bad item from peer: expexted {}, got {}",
            locator_hash, locator_hash_from_peer,
        )));
    }

    // This checks proof validity:
    let object = header.item.object()?;

    header.item.insert()?;

    log::info!("receiving data");
    header
        .object_header
        .recv_data(&mut receiver, object.hash)
        .await?;

    log::info!("done receiving item");

    Ok(object)
}

pub async fn send_item(sender: &ChannelSender, item: CollectionItem) -> Result<(), crate::Error> {
    let object = item.object()?;
    let hash = item.locator().hash();
    let header = ItemHeader::for_item(item)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceHeader::send_negotiate(sender, hash).await?;
    log::info!("sending item header");
    header.send(sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.object_header.send_data(sender, &object).await?;

    log::info!("done sending object");

    Ok(())
}
