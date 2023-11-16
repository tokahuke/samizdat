//! Protocol for information transfer between peers.

use brotli::{CompressorReader, Decompressor};
use chrono::TimeZone;
use futures::prelude::*;
use samizdat_common::MerkleTree;
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_derive::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::time::Duration;

use samizdat_common::cipher::TransferCipher;
use samizdat_common::Hash;

use crate::cli;
use crate::models::{CollectionItem, ObjectMetadata, ObjectRef};
use crate::models::{ContentStream, CHUNK_SIZE};

use super::{ChannelReceiver, ChannelSender};

/// The maximum number of bytes allowed for a header. This puts a hard cap on the maximum file size
/// at around 2.34GB.
const MAX_HEADER_LENGTH: usize = CHUNK_SIZE;
/// The maximum size of the stream, with plenty of room for errors.
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
            .ok_or("channel dried")?;
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

    /// Sends a header from a channel and creates a cipher for all further transmissions.
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

/// Whether to proceed or not after receiving the value of the item object hash.
#[derive(Debug, Serialize, Deserialize)]
enum ProceedMessage {
    /// Proceed with object transmission.
    Proceed,
    /// Object found local; end of transaction.
    Cancel,
}

impl Message for ProceedMessage {}

/// A header sending information (metadata) on an item.
#[derive(Debug, Serialize, Deserialize)]
struct ObjectMessage {
    nonce: Hash,
    metadata: ObjectMetadata,
}

impl Message for ObjectMessage {}

impl ObjectMessage {
    /// Creates an object header for a given object.
    ///
    /// # Panics:
    ///
    /// If object does not exist locally.
    fn for_object(object: &ObjectRef) -> Result<ObjectMessage, crate::Error> {
        let mut metadata = object
            .metadata()?
            .ok_or_else(|| format!("Object message for inexistent object: {object:?}"))?;

        // Need to omit some details before sending through the wire:
        metadata.received_at = chrono::Utc.timestamp_nanos(0);

        Ok(ObjectMessage {
            nonce: Hash::rand(),
            metadata,
        })
    }

    /// Use this header to receive the object from the peer.
    pub fn recv_data(
        self,
        receiver: ChannelReceiver,
        hash: Hash,
        query_duration: Duration,
    ) -> Result<ContentStream, crate::Error> {
        let merkle_tree = MerkleTree::from(self.metadata.hashes.clone());

        // Check if content actually corresponds to advertized hash:
        if merkle_tree.root() != hash {
            return Err(format!(
                "bad content from peer: expected {}, got {}",
                hash,
                merkle_tree.root(),
            )
            .into());
        }

        // Refuse if content is too big:
        if self.metadata.content_size > cli().max_content_size * 1_000_000 {
            return Err(format!(
                "content too big: max size is {}Mb, but content is {}Mb",
                cli().max_content_size,
                self.metadata.content_size / 1_000_000,
            )
            .into());
        }

        // Stream content
        let cipher = TransferCipher::new(&hash, &self.nonce);
        let content_size = self.metadata.content_size;
        let mut transferred_size = 0;
        let content_stream =
            receiver
                .recv_many(MAX_STREAM_SIZE)
                .and_then(move |mut compressed_chunk| {
                    // Decrypt and decompress:
                    cipher.decrypt(&mut compressed_chunk);

                    async move {
                        // Decompress chunk:
                        let mut chunk = Vec::with_capacity(CHUNK_SIZE);
                        Decompressor::new(Cursor::new(compressed_chunk), 4096)
                            .read_to_end(&mut chunk)?;

                        transferred_size += chunk.len();

                        if transferred_size <= content_size {
                            Ok(chunk)
                        } else {
                            Err(format!(
                                "Transferred size of {transferred_size} exceeds advertized size \
                                of {content_size}"
                            )
                            .into())
                        }
                    }
                });

        // Build content from stream (this limits content size to the advertised amount)
        let tee = ObjectRef::import(
            merkle_tree,
            self.metadata,
            query_duration,
            Box::pin(content_stream),
        );

        log::info!("done building object");

        Ok(tee)
    }

    /// Use this header to send the object to the peer.
    pub async fn send_data(
        self,
        sender: &ChannelSender,
        object: &ObjectRef,
    ) -> Result<(), crate::Error> {
        let cipher = TransferCipher::new(object.hash(), &self.nonce);
        let chunks = stream::iter(object.iter_content()?.expect("object exits"));

        // TODO play with concurrent streams later.
        chunks
            .try_for_each_concurrent(Some(1), |chunk| {
                log::debug!("stream for data opened");
                let mut compressed = CompressorReader::new(Cursor::new(chunk), 4096, 4, 22)
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()
                    .expect("never error");
                cipher.encrypt(&mut compressed);

                async move { sender.send(&compressed).await }
            })
            .await?;

        log::info!(
            "finished sending {} to {}",
            object.hash(),
            sender.remote_address()
        );

        Ok(())
    }
}

/// Represents an object in the process of being received. The object to which this
/// struct corresponds most likely does not exist in the database.
pub struct ReceivedObject {
    /// The received, but not verified metadata.
    metadata: ObjectMetadata,
    /// A stream for the incoming content. The content is streamed from the local
    /// database. Therefore it is "proper for consumption".
    content_stream: ContentStream,
    /// The object handle for the soon-to-be object.
    object_ref: ObjectRef,
    /// The time that took for the object to be resolved.
    query_duration: Duration,
}

impl ReceivedObject {
    /// A stream for the incoming content. The content is streamed from the local
    /// database. Therefore it is "proper for consumption".
    pub fn into_content_stream(self) -> ContentStream {
        self.content_stream
    }

    /// The object handle for the soon-to-be object.
    pub fn object_ref(&self) -> ObjectRef {
        self.object_ref.clone()
    }

    /// The received, but not verified metadata.
    pub fn metadata(&self) -> &ObjectMetadata {
        &self.metadata
    }

    /// The time that took for the object to be resolved.
    pub fn query_duration(&self) -> Duration {
        self.query_duration
    }
}

/// Receives the object from a channel.
pub async fn recv_object(
    _sender: ChannelSender,
    mut receiver: ChannelReceiver,
    hash: Hash,
    query_duration: Duration,
) -> Result<ReceivedObject, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, hash).await?;
    log::info!("receiving object header");
    let header = ObjectMessage::recv(&mut receiver, &transfer_cipher).await?;
    let metadata = header.metadata.clone();
    log::info!("receiving data");
    let content_stream = header.recv_data(receiver, hash, query_duration)?;

    log::info!("done receiving object");

    Ok(ReceivedObject {
        metadata,
        content_stream,
        object_ref: ObjectRef::new(hash),
        query_duration,
    })
}

/// Sends an object to a channel.
pub async fn send_object(
    sender: ChannelSender,
    _receiver: ChannelReceiver,
    object: &ObjectRef,
) -> Result<(), crate::Error> {
    let header = ObjectMessage::for_object(object)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(&sender, *object.hash()).await?;
    log::info!("sending object header");
    header.send(&sender, &transfer_cipher).await?;
    log::info!("sending data");
    header.send_data(&sender, object).await?;

    log::info!("done sending object");

    Ok(())
}

/// Represents an item in the process of being received. It can correspond either to a
/// fresh new object or to an existing object in the database.
pub enum ReceivedItem {
    /// The item resolved to a fresh new object not present in th database.
    NewObject(ReceivedObject),
    /// The item resolved to an existing object.
    ExistingObject(ObjectRef),
}

impl ReceivedItem {
    pub fn object_ref(&self) -> ObjectRef {
        match self {
            Self::NewObject(n) => n.object_ref(),
            Self::ExistingObject(e) => e.clone(),
        }
    }
}

/// Receive a collection item from a channel. Returns `Ok(None)` if the item object is
/// perceived to already exist in the database.
pub async fn recv_item(
    sender: ChannelSender,
    mut receiver: ChannelReceiver,
    locator_hash: Hash,
    query_duration: Duration,
) -> Result<ReceivedItem, crate::Error> {
    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, locator_hash).await?;
    log::info!("receiving item header");
    let header = ItemMessage::recv(&mut receiver, &transfer_cipher).await?;

    // No tricks!
    let locator_hash_from_peer = header.item.locator().hash();
    if locator_hash_from_peer != locator_hash {
        return Err(crate::Error::Message(format!(
            "bad item from peer: expected {}, got {}",
            locator_hash, locator_hash_from_peer,
        )));
    }

    // This checks proof validity:
    let object_ref = header.item.object()?;

    // Go away if you already have what you wanted:
    if object_ref.exists()? {
        // Do not attempt to create a `ReceivedObject, because it will attempt to reinsert
        // the object in the database.
        log::info!("Object {} exists. Ending transmission", object_ref.hash());
        // Need to reaffirm the collection-object connection (e.g. the object is there,
        // but is part of another collection and the link is not yet established):
        header.item.insert()?;
        ProceedMessage::Cancel
            .send(&sender, &transfer_cipher)
            .await?;
        return Ok(ReceivedItem::ExistingObject(object_ref));
    } else {
        ProceedMessage::Proceed
            .send(&sender, &transfer_cipher)
            .await?;
    }

    header.item.insert()?;

    log::info!("receiving data");
    let metadata = header.object_header.metadata.clone();
    let content_stream =
        header
            .object_header
            .recv_data(receiver, *object_ref.hash(), query_duration)?;

    log::info!("done receiving item");

    Ok(ReceivedItem::NewObject(ReceivedObject {
        metadata,
        content_stream,
        object_ref,
        query_duration,
    }))
}

/// Sends a collection item to a channel.
pub async fn send_item(
    sender: ChannelSender,
    mut receiver: ChannelReceiver,
    item: CollectionItem,
) -> Result<(), crate::Error> {
    let object = item.object()?;
    let hash = item.locator().hash();
    let header = ItemMessage::for_item(item)?;

    log::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(&sender, hash).await?;
    log::info!("sending item header");
    header.send(&sender, &transfer_cipher).await?;

    log::info!("Receiving proceed message");
    let proceed = ProceedMessage::recv(&mut receiver, &transfer_cipher).await?;

    match proceed {
        ProceedMessage::Proceed => {
            log::info!("sending data");
            header.object_header.send_data(&sender, &object).await?;
        }
        ProceedMessage::Cancel => {
            log::info!("no need to send data");
        }
    }

    log::info!("done sending object");

    Ok(())
}
