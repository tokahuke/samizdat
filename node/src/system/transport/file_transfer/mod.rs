//! Protocol for information transfer between peers.

pub mod legacy;
mod messages;

use std::collections::VecDeque;
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration;

use brotli::{CompressorReader, Decompressor};
use futures::prelude::*;
use samizdat_common::cipher::TransferCipher;
use samizdat_common::{Hash, MerkleTree};
use serde_derive::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::models::{self, ContentStream};
use crate::models::{CollectionItem, ObjectMetadata, ObjectRef};
use crate::utils::{pop_front_chunk, push_front_chunk};

use super::{ChannelReceiver, ChannelSender};

use self::messages::{ItemMessage, Message, NonceMessage, ObjectMessage, MAX_STREAM_SIZE};

const MAX_CONCURRENT_CANDIDATES: usize = 10;
const HASHES_PER_REQUEST: usize = 5;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(5);

struct ValidatedCandidate {
    sender: ChannelSender,
    receiver: ChannelReceiver,
    transfer_cipher: TransferCipher,
    merkle_tree: Option<MerkleTree>,
    metadata: Option<ObjectMetadata>,
    item: Option<CollectionItem>,
}

impl ValidatedCandidate {
    async fn init_object(
        hash: Hash,
        sender: ChannelSender,
        mut receiver: ChannelReceiver,
    ) -> Result<ValidatedCandidate, crate::Error> {
        tracing::info!("negotiating nonce with {}", sender.remote_address());
        let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, hash)
            .await
            .map_err(|err| {
                format!(
                    "Failed to negotiate nonce with {}: {err}",
                    sender.remote_address()
                )
            })?;
        let header = ObjectMessage::recv(&mut receiver, &transfer_cipher)
            .await
            .map_err(|err| {
                format!(
                    "Failed to receive object message from {}: {err}",
                    sender.remote_address()
                )
            })?;
        let merkle_tree = header.validate(hash).map_err(|err| {
            format!(
                "Validation of the object message from {} failed: {err}",
                sender.remote_address()
            )
        })?;

        Ok(ValidatedCandidate {
            sender,
            receiver,
            transfer_cipher,
            merkle_tree: Some(merkle_tree),
            metadata: Some(header.metadata),
            item: None,
        })
    }

    async fn init_item(
        locator_hash: Hash,
        sender: ChannelSender,
        mut receiver: ChannelReceiver,
    ) -> Result<ValidatedCandidate, crate::Error> {
        tracing::info!("negotiating nonce with {}", sender.remote_address());
        let transfer_cipher = NonceMessage::recv_negotiate(&mut receiver, locator_hash)
            .await
            .map_err(|err| {
                format!(
                    "Failed to negotiate nonce with {}: {err}",
                    sender.remote_address()
                )
            })?;
        let item = ItemMessage::recv(&mut receiver, &transfer_cipher)
            .await
            .map_err(|err| {
                format!(
                    "Failed to receive item message from {}: {err}",
                    sender.remote_address()
                )
            })?;
        let merkle_tree = item.validate(locator_hash).map_err(|err| {
            format!(
                "Validation of the object message from {} failed: {err}",
                sender.remote_address()
            )
        })?;

        Ok(ValidatedCandidate {
            sender,
            receiver,
            transfer_cipher,
            merkle_tree: Some(merkle_tree),
            metadata: Some(item.object_header.metadata),
            item: Some(item.item),
        })
    }

    async fn request_chunk(
        &mut self,
        chunks: &[Hash],
        missing_hashes: &mut Vec<Hash>,
        chunk_sender: &mpsc::UnboundedSender<Vec<u8>>,
    ) -> Result<(), crate::Error> {
        RequestChunkMessage::GetChunks(chunks.to_vec())
            .send(&self.sender, &self.transfer_cipher)
            .await?;

        let mut incoming = Box::pin(self.receiver.recv_many(MAX_STREAM_SIZE).take(chunks.len()));

        while let Some(maybe_chunk) = tokio::time::timeout(CHUNK_TIMEOUT, incoming.next())
            .await
            .transpose()
        {
            // Receive chunk:
            let mut compressed_chunk = maybe_chunk
                .map_err(|_| format!("Incoming chunk timed out").into())
                .flatten()?;

            // Move the complicated stuff off the executor
            // Tested! This does make things faster.
            let transfer_cipher = self.transfer_cipher.clone();
            let chunk = tokio::task::spawn_blocking(move || {
                // Decrypt compressed chunk:
                transfer_cipher.decrypt(&mut compressed_chunk);

                // Decompress chunk:
                let mut chunk = Vec::with_capacity(MAX_STREAM_SIZE);
                Decompressor::new(Cursor::new(compressed_chunk), 4096).read_to_end(&mut chunk)?;

                Ok(chunk) as Result<_, crate::Error>
            })
            .await
            .expect("decoing task panicked")?;

            // Check vailidity:
            let received_hash = Hash::from_bytes(&chunk);
            if let Some(position) = missing_hashes
                .iter()
                .position(|hash| hash == &received_hash)
            {
                missing_hashes.remove(position);
                chunk_sender.send(chunk).ok();
            } else {
                return Err(crate::Error::Message(format!(
                    "Received chunk has hash {received_hash}; which was not expected"
                )));
            }
        }

        Ok(())
    }

    async fn say_thanks(self) -> Result<(), crate::Error> {
        RequestChunkMessage::Thanks {}
            .send(&self.sender, &self.transfer_cipher)
            .await
    }
}

struct Hashes {
    hashes: VecDeque<Hash>,
    original_size: usize,
    received: usize,
}

impl Hashes {
    fn new(hashes: Vec<Hash>) -> Hashes {
        Hashes {
            original_size: hashes.len(),
            received: 0,
            hashes: VecDeque::from(hashes),
        }
    }

    fn is_done(&self) -> bool {
        self.original_size == self.received
    }

    fn get_chunk(&mut self) -> Option<Vec<Hash>> {
        if !self.is_done() {
            Some(pop_front_chunk(&mut self.hashes, HASHES_PER_REQUEST))
        } else {
            None
        }
    }

    fn mark_received(&mut self, chunk: Vec<Hash>, missing: Vec<Hash>) {
        self.received += chunk.len() - missing.len();
        push_front_chunk(&mut self.hashes, chunk);
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum RequestChunkMessage {
    GetChunks(Vec<Hash>),
    Thanks,
}

impl Message for RequestChunkMessage {}

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
    candidate_stream: impl 'static + Send + Stream<Item = (ChannelSender, ChannelReceiver)>,
    hash: Hash,
    query_start: Instant,
    deadline_instant: Instant,
) -> Result<ReceivedObject, crate::Error> {
    let mut negotiated = Box::pin(
        stream::select(
            candidate_stream
                .map(move |(sender, receiver)| async move {
                    ValidatedCandidate::init_object(hash, sender, receiver)
                        .await
                        .map_err(|err| tracing::error!("{err}"))
                        .ok()
                })
                .buffer_unordered(MAX_CONCURRENT_CANDIDATES)
                .filter_map(|c| async move { c })
                .map(Ok),
            stream::once(tokio::time::sleep_until(deadline_instant).map(|_| Err(()))),
        )
        .take_while(|c| future::ready(c.is_ok()))
        .map(|c| c.expect("is always ok")),
    );

    // Choose the first peer to do some special things.
    let Ok(maybe_master) = tokio::time::timeout_at(deadline_instant, negotiated.next()).await
    else {
        return Err(format!("Query for {hash} timed out").into());
    };
    let Some(mut master) = maybe_master else {
        return Err(format!("No valid candidate arrived for {hash}").into());
    };

    // Now, the query is considered done.
    let query_duration = Instant::now().duration_since(query_start);

    // Prepare to receive content:
    let (chunk_sender, mut chunk_recv) = mpsc::unbounded_channel();
    let merkle_tree = master.merkle_tree.clone().take().expect("is always set");
    let metadata = master.metadata.take().expect("is always set");
    let hashes = Arc::new(Mutex::new(Hashes::new(merkle_tree.hashes().to_vec())));

    // Receive the content in a separate task:
    tokio::spawn(
        stream::once(async move { master })
            .chain(negotiated)
            .for_each_concurrent(None, move |mut candidate| {
                let hashes = hashes.clone();
                let chunk_sender = chunk_sender.clone();
                async move {
                    loop {
                        let Some(chunk) = hashes.lock().await.get_chunk() else {
                            break;
                        };

                        let mut missing_hashes = chunk.clone();
                        let outcome = candidate
                            .request_chunk(&chunk, &mut missing_hashes, &chunk_sender)
                            .await;
                        hashes.lock().await.mark_received(chunk, missing_hashes);

                        if let Err(err) = outcome {
                            tracing::error!("{err}");
                            break;
                        }
                    }

                    if let Err(err) = candidate.say_thanks().await {
                        tracing::error!("{err}");
                    }
                }
            }),
    );

    // Import the data into the database:
    let content_stream = ObjectRef::import(
        merkle_tree,
        metadata.clone(),
        query_duration,
        stream::poll_fn(move |cx| chunk_recv.poll_recv(cx)).map(Ok),
    );

    tracing::info!("done receiving object");

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
    mut receiver: ChannelReceiver,
    object: &ObjectRef,
) -> Result<(), crate::Error> {
    let header = ObjectMessage::for_object(object)?;

    tracing::info!("negotiating nonce");
    let transfer_cipher = NonceMessage::send_negotiate(&sender, *object.hash()).await?;
    tracing::info!("sending object header");
    header.send(&sender, &transfer_cipher).await?;

    loop {
        match RequestChunkMessage::recv(&mut receiver, &transfer_cipher).await? {
            RequestChunkMessage::Thanks => break,
            RequestChunkMessage::GetChunks(chunks) => {
                for chunk in chunks {
                    if !header.metadata.hashes.contains(&chunk) {
                        Err(format!(
                            "Candidate {} requested chunk {chunk} out of object {}",
                            sender.remote_address(),
                            object.hash(),
                        ))?;
                    }

                    let sender = sender.clone();
                    let transfer_cipher = transfer_cipher.clone();

                    // This doesn't make stuff much faster, but... I did it on the
                    // decoding side, so... why not?
                    let compressed = tokio::task::spawn_blocking(move || {
                        let chunk_content = models::get_chunk(chunk)?;
                        let mut compressed =
                            CompressorReader::new(Cursor::new(chunk_content), 4096, 4, 22)
                                .bytes()
                                .collect::<Result<Vec<_>, _>>()
                                .expect("never error");
                        transfer_cipher.encrypt(&mut compressed);
                        Ok(compressed) as Result<Vec<u8>, crate::Error>
                    })
                    .await
                    .expect("encoding task panicked")?;

                    sender.send(&compressed).await?;
                }
            }
        }
    }

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
    candidate_stream: impl 'static + Send + Stream<Item = (ChannelSender, ChannelReceiver)>,
    locator_hash: Hash,
    query_start: Instant,
    deadline_instant: Instant,
) -> Result<ReceivedItem, crate::Error> {
    let mut negotiated = Box::pin(
        stream::select(
            candidate_stream
                .map(move |(sender, receiver)| async move {
                    ValidatedCandidate::init_item(locator_hash, sender, receiver)
                        .await
                        .map_err(|err| tracing::error!("{err}"))
                        .ok()
                })
                .buffer_unordered(MAX_CONCURRENT_CANDIDATES)
                .filter_map(|c| async move { c })
                .map(Ok),
            stream::once(tokio::time::sleep_until(deadline_instant).map(|_| Err(()))),
        )
        .take_while(|c| future::ready(c.is_ok()))
        .map(|c| c.expect("is always ok")),
    );

    // Choose the first peer to do some special things.
    let Ok(maybe_master) = tokio::time::timeout_at(deadline_instant, negotiated.next()).await
    else {
        return Err(format!("Query for locator {locator_hash} timed out").into());
    };
    let Some(mut master) = maybe_master else {
        return Err(format!("No valid candidate arrived for locator {locator_hash}").into());
    };

    // Now, the query is considered done.
    let query_duration = Instant::now().duration_since(query_start);

    // Prepare to receive data:
    let (chunk_sender, mut chunk_recv) = mpsc::unbounded_channel();
    let merkle_tree = master.merkle_tree.clone().take().expect("is always set");
    let metadata = master.metadata.take().expect("is always set");
    let item = master.item.take().expect("is always set");
    let hashes = Arc::new(Mutex::new(Hashes::new(merkle_tree.hashes().to_vec())));

    // Insert item (should already be validated by this point.)
    item.insert()?;

    // Get object ref:
    let object_ref = ObjectRef::new(merkle_tree.root());

    // Go away if you already have what you wanted:
    if object_ref.exists()? || object_ref.is_null() {
        if object_ref.is_null() {
            tracing::info!("Got null object as response. Ending transmission");
        } else {
            // Do not attempt to create a `ReceivedObject, because it will attempt to reinsert
            // the object in the database.
            tracing::info!("Object {} exists. Ending transmission", object_ref.hash());
        }

        // Ending transmission from all potential candidates that might arrive:
        tokio::spawn(
            stream::once(async move { master })
                .chain(negotiated)
                .for_each_concurrent(None, move |candidate| async move {
                    if let Err(err) = candidate.say_thanks().await {
                        tracing::error!("{err}");
                    }
                }),
        );
        return Ok(ReceivedItem::ExistingObject(object_ref));
    }

    // Receive the content in a separate task:
    tokio::spawn(
        stream::once(async move { master })
            .chain(negotiated)
            .for_each_concurrent(None, move |mut candidate| {
                let hashes = hashes.clone();
                let chunk_sender = chunk_sender.clone();
                async move {
                    loop {
                        let Some(chunk) = hashes.lock().await.get_chunk() else {
                            break;
                        };

                        let mut missing_hashes = chunk.clone();
                        let outcome = candidate
                            .request_chunk(&chunk, &mut missing_hashes, &chunk_sender)
                            .await;
                        hashes.lock().await.mark_received(chunk, missing_hashes);

                        if let Err(err) = outcome {
                            tracing::error!("{err}");
                            break;
                        }
                    }

                    if let Err(err) = candidate.say_thanks().await {
                        tracing::error!("{err}");
                    }
                }
            }),
    );

    // Import the data into the database:
    let content_stream = ObjectRef::import(
        merkle_tree,
        metadata.clone(),
        query_duration,
        stream::poll_fn(move |cx| chunk_recv.poll_recv(cx)).map(Ok),
    );

    tracing::info!("done receiving object");

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
    let header = ItemMessage::for_item(item)?;

    tracing::info!("negotiating nonce");
    let transfer_cipher =
        NonceMessage::send_negotiate(&sender, header.item.locator().hash()).await?;
    tracing::info!("sending object header");
    header.send(&sender, &transfer_cipher).await?;

    loop {
        match RequestChunkMessage::recv(&mut receiver, &transfer_cipher).await? {
            RequestChunkMessage::Thanks => break,
            RequestChunkMessage::GetChunks(chunks) => {
                for &chunk in &chunks {
                    if !header.object_header.metadata.hashes.contains(&chunk) {
                        return Err(format!(
                            "Candidate {} requested chunk {chunk} out of item {}",
                            sender.remote_address(),
                            header.item.locator(),
                        )
                        .into());
                    }

                    let transfer_cipher = transfer_cipher.clone();

                    // This doesn't make stuff much faster, but... I did it on the
                    // decoding side, so... why not?
                    let compressed = tokio::task::spawn_blocking(move || {
                        let chunk_content = models::get_chunk(chunk)?;
                        let mut compressed =
                            CompressorReader::new(Cursor::new(chunk_content), 4096, 4, 22)
                                .bytes()
                                .collect::<Result<Vec<_>, _>>()
                                .expect("never error");
                        transfer_cipher.encrypt(&mut compressed);
                        Ok(compressed) as Result<Vec<u8>, crate::Error>
                    })
                    .await
                    .expect("encoding task panicked")?;

                    sender.send(&compressed).await?;
                }
            }
        }
    }

    Ok(())
}
