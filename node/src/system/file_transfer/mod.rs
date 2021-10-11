//! Protocol for information transfer between peers.

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

/// The maximum number of bytes allowed for a header.
const MAX_HEADER_LENGTH: usize = 4_096;
/// The maximum size of the stream.
const MAX_STREAM_SIZE: usize = crate::models::CHUNK_SIZE * 2;

/// A header that can be sent from the sender to the receiver _before_ the stream starts.
#[async_trait::async_trait]
trait Message: 'static + Send + Sync + SerdeSerialize + for<'a> SerdeDeserialize<'a> {
    /// Receive the header.
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

    /// Send the header.
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

/// Sends a _nonce_ (a number used only once) that will be used for deriving a key to transfer
/// the stream.
#[derive(Default, Serialize, Deserialize)]
struct NonceMessage {
    nonce: Hash,
}

impl Message for NonceMessage {}

impl NonceMessage {
    fn new() -> NonceMessage {
        NonceMessage {
            nonce: Hash::rand(),
        }
    }

    /// Combines with a content header to create a symmetric cipher.
    fn cipher(self, hash: Hash) -> TransferCipher {
        TransferCipher::new(&hash, &self.nonce)
    }

    /// Receive a header from a channel and creates a cipher for all further transmissions.
    async fn recv_negotiate(
        receiver: &mut ChannelReceiver,
        hash: Hash,
    ) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceMessage::default().cipher(hash);
        let nonce_header = NonceMessage::recv(receiver, &init_cipher).await?;

        Ok(nonce_header.cipher(hash))
    }

    /// Sendes a header from a channel and creates a cipher for all further transmissions.
    async fn send_negotiate(
        sender: &ChannelSender,
        hash: Hash,
    ) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceMessage::default().cipher(hash);
        let nonce_header = NonceMessage::new();
        nonce_header.send(sender, &init_cipher).await?;

        Ok(nonce_header.cipher(hash))
    }
}

/// A header sending information (metadata) on a collection item.
#[derive(Debug, Serialize, Deserialize)]
struct ItemMessage {
    item: CollectionItem,
    object_header: ObjectMessage,
}

impl Message for ItemMessage {}

impl ItemMessage {
    /// Creates an item header for a given collection item.
    fn for_item(item: CollectionItem) -> Result<ItemMessage, crate::Error> {
        let object_header = ObjectMessage::for_object(&item.object()?)?;
        Ok(ItemMessage {
            item,
            object_header,
        })
    }
}

/// A header sending information (metadata) on an item.
#[derive(Debug, Serialize, Deserialize)]
struct ObjectMessage {
    nonce: Hash,
    content_size: usize,
}

impl Message for ObjectMessage {}

impl ObjectMessage {
    /// Creates an object heade for a given object.
    ///
    /// # Panics:
    ///
    /// If object does not exist locally.
    fn for_object(object: &ObjectRef) -> Result<ObjectMessage, crate::Error> {
        let metadata = object.metadata()?.expect("object exists");

        Ok(ObjectMessage {
            nonce: Hash::rand(),
            content_size: metadata.content_size,
        })
    }

    /// Use this header to receive the object from the peer.
    pub async fn recv_data(
        self,
        receiver: &mut ChannelReceiver,
        hash: Hash,
    ) -> Result<ObjectRef, crate::Error> {
        let cipher = Arc::new(TransferCipher::new(&hash, &self.nonce));

        // Refuse if content is too big:
        if self.content_size > cli().max_content_size * 1_000_000 {
            return Err(format!(
                "content too big: max size is {}, advertised was {}",
                cli().max_content_size * 1_000_000,
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
        let (metadata, object) =
            ObjectRef::import(self.content_size, false, Box::pin(content_stream)).await?;

        log::info!("done building object");

        // Check if the peer is up to any extra sneaky tricks.
        if metadata.content_size != self.content_size {
            Err(format!(
                "actual data length did not match content-size: expected {}, got {}",
                metadata.content_size, self.content_size
            )
            .into())
        } else if *object.hash() != hash {
            Err(format!(
                "bad content from peer: expected {}, got {}",
                hash,
                object.hash(),
            )
            .into())
        } else {
            log::info!("received valid object from peer");
            Ok(object)
        }
    }

    /// Use this header to send the object to the peer.
    pub async fn send_data(
        self,
        sender: &ChannelSender,
        object: &ObjectRef,
    ) -> Result<(), crate::Error> {
        let cipher = TransferCipher::new(object.hash(), &self.nonce);

        for chunk in object.chunks()?.expect("object exits") {
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
            object.hash(),
            sender.remote_address()
        );

        Ok(())
    }
}

/// Receives the object from a channel.
pub async fn recv_object(
    mut receiver: ChannelReceiver,
    hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, hash).await?;
    log::info!("receiving object header");
    let header = ObjectMessage::recv(&mut receiver, &transfer_cipher).await?;
    log::info!("receiving data");
    let object = header.recv_data(&mut receiver, hash).await?;

    log::info!("done receiving object");

    Ok(object)
}

/// Sends an object to a channel.
pub async fn send_object(sender: &ChannelSender, object: &ObjectRef) -> Result<(), crate::Error> {
    object.touch()?;

    let header = ObjectMessage::for_object(object)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(sender, *object.hash()).await?;
    log::info!("sending object header");
    header.send(sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.send_data(sender, object).await?;

    log::info!("done sending object");

    Ok(())
}

/// Receive a collection item from a channel.
///
/// TODO: make object transfer optional if the receiver perceives that it
/// already has the object (one simple table lookup, no seqscan here). This is
/// important as people update their collections often, but keep most of it
/// intact.
pub async fn recv_item(
    mut receiver: ChannelReceiver,
    locator_hash: Hash,
) -> Result<ObjectRef, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, locator_hash).await?;
    log::info!("receiving item header");
    let header = ItemMessage::recv(&mut receiver, &transfer_cipher).await?;

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
        .recv_data(&mut receiver, *object.hash())
        .await?;

    log::info!("done receiving item");

    Ok(object)
}

/// Sends a collection item to a channel.
pub async fn send_item(sender: &ChannelSender, item: CollectionItem) -> Result<(), crate::Error> {
    let object = item.object()?;
    let hash = item.locator().hash();
    let header = ItemMessage::for_item(item)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(sender, hash).await?;
    log::info!("sending item header");
    header.send(sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.object_header.send_data(sender, &object).await?;

    log::info!("done sending object");

    Ok(())
}
