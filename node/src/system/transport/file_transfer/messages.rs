//! Wire messages for the file-transfer protocol: the header an object
//! sender ships before chunks flow, the per-chunk request frames, and
//! the validate-against-merkle entry point the receiver uses.

use brotli::{CompressorReader, Decompressor};
use chrono::TimeZone;
use futures::prelude::*;
use samizdat_common::{MerkleTree, db::readonly_tx};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_derive::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read},
    time::Duration,
};

use samizdat_common::{Hash, cipher::TransferCipher};

use crate::{
    models::{CHUNK_SIZE, CollectionItem, ContentStream, ObjectMetadata, ObjectRef},
    system::transport::{ChannelReceiver, ChannelSender},
};

/// The maximum number of bytes allowed for a header. This puts a hard cap on the maximum
/// file size
/// at around 2.34GB.
pub const MAX_HEADER_LENGTH: usize = CHUNK_SIZE;
/// The maximum size of the stream, with plenty of room for errors.
pub const MAX_STREAM_SIZE: usize = crate::models::CHUNK_SIZE * 2;

/// A header that can be sent from the sender to the receiver _before_ the stream starts.
pub trait Message: 'static + Send + Sync + SerdeSerialize + for<'a> SerdeDeserialize<'a> {
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
        cipher.decrypt(&mut serialized_header)?;
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
        cipher.encrypt(&mut serialized_header)?;
        sender.send(&serialized_header).await?;

        Ok(())
    }
}

/// Sends a _nonce_ (a number used only once) that will be used for deriving a key to
/// transfer
/// the stream.
#[derive(Default, Serialize, Deserialize)]
pub struct NonceMessage {
    nonce: Hash,
}

impl Message for NonceMessage {}

impl NonceMessage {
    pub fn new() -> NonceMessage {
        NonceMessage {
            nonce: Hash::rand(),
        }
    }

    /// Combines with a content header to create a symmetric cipher.
    pub fn cipher(self, hash: Hash) -> TransferCipher {
        TransferCipher::new(&hash, &self.nonce)
    }

    /// Receive a header from a channel and creates a cipher for all further
    /// transmissions.
    pub async fn recv_negotiate(
        receiver: &mut ChannelReceiver,
        hash: Hash,
    ) -> Result<TransferCipher, crate::Error> {
        let init_cipher = NonceMessage::default().cipher(hash);
        let nonce_header = NonceMessage::recv(receiver, &init_cipher).await?;

        Ok(nonce_header.cipher(hash))
    }

    /// Sends a header from a channel and creates a cipher for all further transmissions.
    pub async fn send_negotiate(
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
pub struct ItemMessage {
    pub item: CollectionItem,
    pub object_header: ObjectMessage,
}

impl Message for ItemMessage {}

impl ItemMessage {
    /// Creates an item header for a given collection item.
    pub fn for_item(item: CollectionItem) -> Result<ItemMessage, crate::Error> {
        let object_header = ObjectMessage::for_object(&item.object()?)?;
        Ok(ItemMessage {
            item,
            object_header,
        })
    }

    pub fn validate(&self, locator_hash: Hash) -> Result<MerkleTree, crate::Error> {
        let locator_hash_from_peer = self.item.locator().hash();
        if locator_hash_from_peer != locator_hash {
            return Err(crate::Error::Message(format!(
                "bad item from peer: expected locator hash {}, got {}",
                locator_hash, locator_hash_from_peer,
            )));
        }

        // Calling `.object()?` validates the inclusion proof.
        self.object_header.validate(*self.item.object()?.hash())
    }
}

/// A header sending information (metadata) on an item.
#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectMessage {
    pub nonce: Hash,
    pub metadata: ObjectMetadata,
}

impl Message for ObjectMessage {}

impl ObjectMessage {
    /// Creates an object header for a given object.
    ///
    /// # Panics:
    ///
    /// If object does not exist locally.
    pub fn for_object(object: &ObjectRef) -> Result<ObjectMessage, crate::Error> {
        if object.is_null() {
            return Ok(ObjectMessage {
                nonce: Hash::rand(),
                metadata: ObjectMetadata::for_null_object(),
            });
        }

        let mut metadata = readonly_tx(|tx| object.metadata(tx))?
            .ok_or_else(|| format!("Object message for inexistent object: {object:?}"))?;

        // Need to omit some details before sending through the wire:
        metadata.received_at = chrono::Utc.timestamp_nanos(0);

        Ok(ObjectMessage {
            nonce: Hash::rand(),
            metadata,
        })
    }

    /// Validates that this message's merkle tree resolves to `hash`,
    /// returning the trusted merkle tree or a validation error.
    ///
    /// The per-object size cap (formerly here) now lives in
    /// `ObjectRef::import`, composed with the per-subscription and
    /// node-wide caps under a single atomic reserve. See
    /// `node/src/cap.rs::OBJECT_SIZE_LIMIT`.
    pub fn validate(&self, hash: Hash) -> Result<MerkleTree, crate::Error> {
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

        Ok(merkle_tree)
    }

    /// Use this header to receive the object from the peer.
    #[allow(unused)]
    pub fn recv_data(
        self,
        receiver: ChannelReceiver,
        hash: Hash,
        query_duration: Duration,
    ) -> Result<ContentStream, crate::Error> {
        let merkle_tree = self.validate(hash)?;

        // Stream content
        let cipher = TransferCipher::new(&hash, &self.nonce);
        let content_size = self.metadata.content_size;
        let mut transferred_size = 0;
        let content_stream =
            receiver
                .recv_many_owned(MAX_STREAM_SIZE)
                .map(move |compressed_chunk| {
                    compressed_chunk.and_then(|mut compressed_chunk| {
                        // Decrypt and decompress:
                        cipher.decrypt(&mut compressed_chunk)?;

                        // Decompress chunk with a hard cap on the OUTPUT length.
                        // Without `.take()`, a brotli zip-bomb (a few KB
                        // compressed that inflates to 1+ TB) would fill memory
                        // here before the post-decompress
                        // `transferred_size > content_size` check below could
                        // fire. Reading one byte past CHUNK_SIZE lets us
                        // distinguish "decompressed to CHUNK_SIZE" from
                        // "source wanted more."
                        let mut chunk = Vec::with_capacity(CHUNK_SIZE);
                        let mut limited = Decompressor::new(Cursor::new(compressed_chunk), 4096)
                            .take((CHUNK_SIZE as u64) + 1);
                        limited.read_to_end(&mut chunk)?;
                        if chunk.len() > CHUNK_SIZE {
                            return Err(format!(
                                "decompressed chunk exceeds CHUNK_SIZE \
                                 ({} bytes); rejecting (zip-bomb defense)",
                                chunk.len(),
                            )
                            .into());
                        }

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
                    })
                });

        // Build content from stream (this limits content size to the advertised amount).
        // `caller_cap` is None at this layer; the subscription-side caller passes its
        // cap through the higher-level fetch loop.
        let tee = ObjectRef::import(
            merkle_tree,
            self.metadata,
            query_duration,
            Box::pin(content_stream),
            None,
        )?;

        tracing::info!("done building object");

        Ok(tee)
    }

    /// Use this header to send the object to the peer.
    #[allow(unused)]
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
                tracing::debug!("stream for data opened");
                // Source is `Cursor::new(chunk)` -- already in memory, so the
                // clippy::unbuffered_bytes premise does not apply.
                #[allow(clippy::unbuffered_bytes)]
                let mut compressed = CompressorReader::new(Cursor::new(chunk), 4096, 4, 22)
                    .bytes()
                    .collect::<Result<Vec<_>, _>>()
                    .expect("never error");
                let encrypt_result = cipher.encrypt(&mut compressed);

                async move {
                    encrypt_result?;
                    sender.send(&compressed).await
                }
            })
            .await?;

        tracing::info!(
            "finished sending {} to {}",
            object.hash(),
            sender.remote_address()
        );

        Ok(())
    }
}
