//! Protocol for information transfer between peers.

mod messages;

use std::{
    collections::VecDeque,
    io::{Cursor, Read},
    pin::pin,
    sync::Arc,
    time::Duration,
};

use brotli::{CompressorReader, Decompressor};
use futures::{prelude::*, stream::BoxStream};
use samizdat_common::{
    Hash, MerkleTree,
    cipher::TransferCipher,
    db::{readonly_tx, writable_tx},
};
use serde_derive::{Deserialize, Serialize};
use tokio::{
    sync::{Mutex, mpsc},
    time::Instant,
};

use crate::{
    models::{self, CHUNK_SIZE, CollectionItem, ObjectMetadata, ObjectRef},
    utils::{pop_front_chunk, push_front_chunk},
};

use super::{ChannelReceiver, ChannelSender};

use self::messages::{ItemMessage, MAX_STREAM_SIZE, Message, NonceMessage, ObjectMessage};

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

        let mut incoming = pin!(self.receiver.recv_many(MAX_STREAM_SIZE).take(chunks.len()));

        while let Some(maybe_chunk) = tokio::time::timeout(CHUNK_TIMEOUT, incoming.next())
            .await
            .transpose()
        {
            // Receive chunk:
            let mut compressed_chunk = maybe_chunk
                .map_err(|_| "Incoming chunk timed out".to_string().into())
                .flatten()?;

            // Move the complicated stuff off the executor
            // Tested! This does make things faster.
            let transfer_cipher = self.transfer_cipher.clone();
            let chunk = tokio::task::spawn_blocking(move || {
                // Decrypt compressed chunk:
                transfer_cipher.decrypt(&mut compressed_chunk)?;

                // Decompress chunk, with a hard cap on the OUTPUT length.
                // Without this, a brotli zip-bomb (a few KB compressed that
                // inflates to 1+ TB) would blow up here before any downstream
                // chunk-size check could fire. Reading one byte past CHUNK_SIZE
                // lets us distinguish "decompressed to exactly CHUNK_SIZE" from
                // "the source wanted more"; the latter we reject.
                let mut chunk = Vec::with_capacity(CHUNK_SIZE);
                let mut limited = Decompressor::new(Cursor::new(compressed_chunk), 4096)
                    .take((CHUNK_SIZE as u64) + 1);
                limited.read_to_end(&mut chunk)?;
                if chunk.len() > CHUNK_SIZE {
                    return Err(format!(
                        "decompressed chunk exceeds CHUNK_SIZE ({} bytes); \
                         rejecting (zip-bomb defense)",
                        chunk.len(),
                    )
                    .into());
                }

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
        // `>=` rather than `==`: the torrent-style design has multiple peers
        // serving the same chunks for redundancy, so `received` can overshoot
        // `original_size` when more than one peer delivers a full set. With
        // strict equality, the first complete delivery flips done briefly and
        // then any subsequent duplicate flips it back off forever; candidate
        // tasks then loop sending empty `GetChunks` requests until QUIC drops.
        self.received >= self.original_size
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

/// Lifecycle state: an object whose merkle tree has been validated
/// against the requested hash and whose chunks are streaming in, but
/// which has not yet been committed to storage. The caller decides
/// when to import (and with which cap) by passing these raw materials
/// to `ObjectRef::import`.
pub struct InFlightObject {
    pub merkle_tree: MerkleTree,
    pub metadata: ObjectMetadata,
    pub object_ref: ObjectRef,
    pub query_duration: Duration,
    pub chunks: BoxStream<'static, Result<Vec<u8>, crate::Error>>,
}

/// Receives the object from a channel. Returns an `InFlightObject`
/// with the raw materials (merkle tree, metadata, chunk stream) that
/// the caller must hand to `ObjectRef::import` along with their
/// chosen cap. Network-layer code does not commit to storage on its
/// own.
pub async fn recv_object(
    candidate_stream: impl 'static + Send + Stream<Item = (ChannelSender, ChannelReceiver)>,
    hash: Hash,
    query_start: Instant,
    deadline_instant: Instant,
) -> Result<InFlightObject, crate::Error> {
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
    let merkle_tree = master.merkle_tree.clone().expect("is always set");
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

    tracing::info!("done negotiating object; chunks streaming");

    Ok(InFlightObject {
        merkle_tree,
        metadata,
        object_ref: ObjectRef::new(hash),
        query_duration,
        chunks: stream::poll_fn(move |cx| chunk_recv.poll_recv(cx))
            .map(Ok)
            .boxed(),
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
                        let chunk_content = readonly_tx(|tx| models::get_chunk(tx, chunk))?;
                        // Source is `Cursor::new(chunk_content)` -- already in
                        // memory, so the clippy::unbuffered_bytes premise does
                        // not apply.
                        #[allow(clippy::unbuffered_bytes)]
                        let mut compressed =
                            CompressorReader::new(Cursor::new(chunk_content), 4096, 4, 22)
                                .bytes()
                                .collect::<Result<Vec<_>, _>>()
                                .expect("never error");
                        transfer_cipher.encrypt(&mut compressed)?;
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

/// Outcome of a fetch: either the object is already present locally,
/// or an in-flight object is streaming in and awaiting import by the
/// caller.
pub enum FetchOutcome {
    /// The fetch resolved to an object that already exists in the
    /// database; no import is needed.
    Existing(ObjectRef),
    /// The fetch produced a freshly-validated incoming stream; the
    /// caller must call `ObjectRef::import` with their chosen cap.
    InFlight(InFlightObject),
}

impl FetchOutcome {
    pub fn object_ref(&self) -> ObjectRef {
        match self {
            Self::InFlight(in_flight) => in_flight.object_ref.clone(),
            Self::Existing(object_ref) => object_ref.clone(),
        }
    }
}

/// Receive a collection item from a channel. Returns a `FetchOutcome`
/// distinguishing the already-cached case from the in-flight case;
/// the caller decides when (and with which cap) to call
/// `ObjectRef::import` on the in-flight variant.
pub async fn recv_item(
    candidate_stream: impl 'static + Send + Stream<Item = (ChannelSender, ChannelReceiver)>,
    locator_hash: Hash,
    query_start: Instant,
    deadline_instant: Instant,
) -> Result<FetchOutcome, crate::Error> {
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
    let merkle_tree = master.merkle_tree.clone().expect("is always set");
    let metadata = master.metadata.take().expect("is always set");
    let item = master.item.take().expect("is always set");
    let hashes = Arc::new(Mutex::new(Hashes::new(merkle_tree.hashes().to_vec())));

    // Insert item (should already be validated by this point.)
    //
    // The item is inserted BEFORE the underlying object is fully downloaded; i.e.,
    // between this write and the eventual completion of the chunk stream, the item row
    // points at a non-existent object. This is deliberate (it lets concurrent local
    // lookups see "in progress" rather than "missing") and is safe because:
    //   * `resolve_object` always re-checks `object.exists(tx)` before serving; misses fall
    //     through to the network query path rather than panicking.
    //   * `vacuum::drop_dangling_items` reaps items that never see their object arrive.
    //   * `CollectionItem::object()` only returns Err on inclusion-proof failure (a
    //     structural check), not on object-existence; existence is handled downstream.
    writable_tx(|tx| {
        item.insert(tx)?;
        Ok(())
    })
    .expect("infallible");

    // Get object ref:
    let object_ref = ObjectRef::new(merkle_tree.root());

    // Go away if you already have what you wanted:
    if readonly_tx(|tx| object_ref.exists(tx))? || object_ref.is_null() {
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
        return Ok(FetchOutcome::Existing(object_ref));
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

    tracing::info!("done negotiating item; chunks streaming");

    Ok(FetchOutcome::InFlight(InFlightObject {
        merkle_tree,
        metadata,
        object_ref,
        query_duration,
        chunks: stream::poll_fn(move |cx| chunk_recv.poll_recv(cx))
            .map(Ok)
            .boxed(),
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
                        let chunk_content = readonly_tx(|tx| models::get_chunk(tx, chunk))?;
                        // Source is `Cursor::new(chunk_content)` -- already in
                        // memory, so the clippy::unbuffered_bytes premise does
                        // not apply.
                        #[allow(clippy::unbuffered_bytes)]
                        let mut compressed =
                            CompressorReader::new(Cursor::new(chunk_content), 4096, 4, 22)
                                .bytes()
                                .collect::<Result<Vec<_>, _>>()
                                .expect("never error");
                        transfer_cipher.encrypt(&mut compressed)?;
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
