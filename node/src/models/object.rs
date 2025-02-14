//! Objects are files in the Samizdat network that are uniquely identified by their
//! hash. Objects are powered by Merkle trees to allow torrent-like download and better
//! storage of similar content.

use chrono::{DateTime, TimeZone, Utc};
use futures::prelude::*;
use samizdat_common::db::{readonly_tx, writable_tx, Droppable, Table as _, TxHandle, WritableTx};
use serde_derive::{Deserialize, Serialize};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{collections::BTreeMap, convert::TryInto};
use tokio::sync::mpsc;

use samizdat_common::{Hash, Hint, MerkleTree, Riddle};

use crate::db::{MergeOperation, Table};

use super::{Bookmark, BookmarkType};

/// The size of a chunk. An object consists of a sequence of chunks, the hash
/// of which are used to create the Merkle tree whose root hash is the object
/// hash.
pub const CHUNK_SIZE: usize = 256_000;

/// The first section before the actual content of the object. The header is
/// encoded as a null-escaped byte sequence in the beginning of the first chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectHeader {
    /// The MIME type of this object.
    content_type: String,
    /// Whether this is a draft object or not. Draft objects cannot be shared
    /// publicly.
    is_draft: bool,
    /// A number with no semantics whatsoever. You can use this to create a
    /// different object hash for the same content.
    pub nonce: u64,
}

impl ObjectHeader {
    /// Creates a new object header.
    pub fn new(content_type: String, is_draft: bool) -> Result<ObjectHeader, crate::Error> {
        Ok(ObjectHeader {
            content_type,
            is_draft,
            nonce: 0,
        })
    }

    /// The MIME type of this object.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// Whether this is a draft object or not. Draft objects cannot be shared
    /// publicly.
    pub fn is_draft(&self) -> bool {
        self.is_draft
    }

    /// Creates a new object header that contains the same information as the current
    /// header, but changes the nonce. This allows objects of the same content to be
    /// issued under different hashes.
    pub fn reissue(&self) -> ObjectHeader {
        ObjectHeader {
            content_type: self.content_type.clone(),
            is_draft: self.is_draft,
            nonce: rand::random(),
        }
    }

    /// Reads a header from an iterator of bytes.
    pub fn read(
        into_iter: impl IntoIterator<Item = Result<u8, crate::Error>>,
    ) -> Result<(usize, ObjectHeader), crate::Error> {
        let mut buffer = Vec::new();
        let mut read = 0;

        let iter = into_iter.into_iter();
        let limited = iter.take(CHUNK_SIZE);
        let mut is_maybe_quoted = false;

        for byte in limited {
            read += 1;
            let byte = byte?;
            let curr_is_null = byte == 0;
            match (is_maybe_quoted, curr_is_null) {
                // Found quote
                (true, true) => {
                    buffer.push(0);
                    is_maybe_quoted = false;
                }
                // Found end
                (true, false) => break,
                // Found byte
                (false, false) => {
                    buffer.push(byte);
                }
                // Found _possible_ quote
                (false, true) => {
                    is_maybe_quoted = true;
                }
            }
        }

        Ok((read, bincode::deserialize(&buffer)?))
    }

    /// Creates the null-encoded sequence of bytes for this header.
    pub fn buffer(&self) -> Vec<u8> {
        let serialized = bincode::serialize(self).expect("can serialize");
        let mut buffer = Vec::with_capacity(2 * serialized.len() + 1);

        // Escape:
        for byte in serialized {
            if byte == 0 {
                buffer.extend([0, 0]);
            } else {
                buffer.push(byte);
            }
        }

        buffer.push(0);
        buffer.push(1);

        buffer
    }
}

/// Helper function to get a chunk by its hash in the database.
pub fn get_chunk<Tx: TxHandle>(tx: &Tx, hash: Hash) -> Result<Vec<u8>, crate::Error> {
    Ok(Table::ObjectChunks
        .get(tx, hash, |slice| slice.to_vec())
        .ok_or_else(|| format!("Chunk missing: {}", hash))?)
}

/// Information about the object that is "out of band", that is, does not compose the hash
/// directly. This is used for internal bookkeeping inside the node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMetadata {
    /// The hashes of each chunk in the order that they appear.
    pub hashes: Vec<Hash>,
    /// This field is informational and for convenience only. The _real_ header is in the
    /// first bytes of the first chunk.
    pub header: ObjectHeader,
    /// Sum of the sizes of all chunks. This includes the header size.
    pub content_size: usize,
    /// The timestamp this object was received on. This field is not transmitted through the network.
    pub received_at: chrono::DateTime<chrono::Utc>,
}

impl ObjectMetadata {
    pub fn for_null_object() -> ObjectMetadata {
        ObjectMetadata {
            hashes: vec![Hash::zero()],
            header: ObjectHeader {
                content_type: "application/x-empty".to_owned(),
                is_draft: false,
                nonce: 0,
            },
            content_size: 0,
            received_at: chrono::Utc.timestamp_nanos(0),
        }
    }
}

/// An iterator over the bytes of an object, including its header.
#[must_use]
pub struct BytesIter {
    /// An iterator over hashes.
    hashes: std::vec::IntoIter<Hash>,
    /// An iterator over the current chunk.
    current_chunk: Option<std::vec::IntoIter<u8>>,
    /// Indicates whether an error has occurred.
    is_error: bool,
    /// Indicates whether an object header must be skipped for the next chunk.
    skip_header: bool,
}

impl Iterator for BytesIter {
    type Item = Result<u8, crate::Error>;
    fn next(&mut self) -> Option<Result<u8, crate::Error>> {
        // Fused on error:
        if self.is_error {
            return None;
        }

        // Try get running chunk:
        if let Some(chunk) = self.current_chunk.as_mut() {
            if let Some(byte) = chunk.next() {
                return Some(Ok(byte));
            }
        }

        // Try get new chunk:
        if let Some(hash) = self.hashes.next() {
            match readonly_tx(|tx| get_chunk(tx, hash)) {
                // Found chunk? Load an try again!
                Ok(chunk) => {
                    let mut iter = chunk.into_iter();

                    // If an object header must be skipped, then skip it!
                    if self.skip_header {
                        let (_, _) = ObjectHeader::read((&mut iter).map(Ok)).unwrap();
                        self.skip_header = false;
                    }

                    self.current_chunk = Some(iter);

                    return self.next();
                }
                // Found error? Return error and fuse.
                Err(error) => {
                    self.is_error = true;
                    return Some(Err(error));
                }
            }
        }

        // Exhausted
        None
    }
}

/// An iterator over the chunks of an object.
#[must_use]
pub struct ContentIter {
    /// An iterator over hashes.
    hashes: std::vec::IntoIter<Hash>,
    /// Indicates whether an error has occurred.
    is_error: bool,
}

impl Iterator for ContentIter {
    type Item = Result<Vec<u8>, crate::Error>;
    fn next(&mut self) -> Option<Result<Vec<u8>, crate::Error>> {
        // Fused on error:
        if self.is_error {
            return None;
        }

        // Try get new chunk:
        if let Some(hash) = self.hashes.next() {
            match readonly_tx(|tx| get_chunk(tx, hash)) {
                // Found chunk? Yield.
                Ok(chunk) => {
                    return Some(Ok(chunk));
                }
                // Found error? Return error and fuse.
                Err(error) => {
                    self.is_error = true;
                    return Some(Err(error));
                }
            }
        }

        // Exhausted
        None
    }
}

/// A stream over the chunks of an object.
#[must_use]
pub struct ContentStream {
    /// A stream over the chunk hashes, in order.
    hashes: Pin<Box<dyn Send + Stream<Item = Result<Hash, crate::Error>>>>,
    /// Indicates whether an error has occurred.
    is_error: bool,
    /// Indicates whether an object header must be skipped for the next chunk.
    skip_header: bool,
}

impl Stream for ContentStream {
    type Item = Result<Vec<u8>, crate::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Fused on error:
        if self.is_error {
            return Poll::Ready(None);
        }

        // Try getting new chunk.
        let polled_chunk = Pin::new(&mut self.hashes)
            .poll_next(cx)
            .map(|hash| hash.map(|hash| hash.and_then(|h| readonly_tx(|tx| get_chunk(tx, h)))));

        match polled_chunk {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(Err(err))) => {
                self.is_error = true;
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Some(Ok(chunk))) => {
                // If an object header must be skipped, then skip it!
                let chunk = if self.skip_header {
                    let mut iter = chunk.into_iter();
                    ObjectHeader::read((&mut iter).map(Ok))?;
                    self.skip_header = false;

                    iter.collect()
                } else {
                    chunk
                };

                Poll::Ready(Some(Ok(chunk)))
            }
        }
    }
}

impl ContentStream {
    /// Collects all bytes read for this content stream.
    pub async fn collect_content(mut self) -> Result<Vec<u8>, crate::Error> {
        let mut content = vec![];

        while let Some(chunk) = self.next().await.transpose()? {
            content.extend(chunk);
        }

        Ok(content)
    }
}

/// The null object. An object that is guaranteed to return a 404 not found.
pub const NULL_OBJECT: ObjectRef = ObjectRef { hash: Hash::zero() };

/// A handle to an object. The object does not necessarily needs to exist in the database.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectRef {
    /// The hash that defines this object.
    hash: Hash,
}

impl Droppable for ObjectRef {
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        tracing::info!("Removing object {:?}", self);

        let Some(metadata) = self.metadata(tx)? else {
            // Object does not exist.
            return Ok(());
        };

        // // Never do this! You risk corrupting unrelated objects.
        // for hash in &metadata.hashes {
        //     batch.delete_cf(Table::ObjectChunks.get(), hash);
        // }

        for chunk_hash in metadata.hashes {
            Table::ObjectChunkRefCount.map(tx, chunk_hash, MergeOperation::Increment(-1).merger());
        }

        // leave the vacuum daemon to clean up unused chunks. It runs frequently.

        self.bookmark(BookmarkType::Reference).clear(tx);
        self.bookmark(BookmarkType::User).clear(tx);

        Table::ObjectStatistics.delete(tx, self.hash);
        Table::ObjectMetadata.delete(tx, self.hash);
        Table::ObjectMetadata.delete(tx, self.hash);
        Table::Objects.delete(tx, self.hash);

        Ok(())
    }
}

impl ObjectRef {
    /// Creates a new object reference from a hash.
    pub fn new(hash: Hash) -> ObjectRef {
        ObjectRef { hash }
    }

    /// Returns the hash associated with this object.
    pub fn hash(&self) -> &Hash {
        &self.hash
    }

    pub fn is_null(&self) -> bool {
        self == &NULL_OBJECT
    }

    /// Returns whether an object exists in the database or not.
    pub fn exists<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        Ok(Table::ObjectMetadata.has(tx, self.hash))
    }

    /// Returns the metadata on this object. This function returns `Ok(None)` if the object
    /// does not actually exist.
    pub fn metadata<Tx: TxHandle>(&self, tx: &Tx) -> Result<Option<ObjectMetadata>, crate::Error> {
        Ok(Table::ObjectMetadata
            .get(tx, self.hash, |serialized| bincode::deserialize(serialized))
            .transpose()?)
    }

    /// Gets statistics on this object. Returns `Ok(None)` if the object does not exist.
    pub fn statistics<Tx: TxHandle>(
        &self,
        tx: &Tx,
    ) -> Result<Option<ObjectStatistics>, crate::Error> {
        Ok(Table::ObjectStatistics
            .get(tx, self.hash, |serialized| bincode::deserialize(serialized))
            .transpose()?)
    }

    /// Update statistics indicating that this object was used. This will signal to the
    /// vacuum daemon that this object is useful and therefore a worse candidate for deletion.
    ///
    /// This function has no effect if the object does not exist.
    fn touch(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        let maybe_statistics: Option<ObjectStatistics> = Table::ObjectStatistics
            .get(tx, self.hash, |serialized| bincode::deserialize(serialized))
            .transpose()?;

        if let Some(mut statistics) = maybe_statistics {
            statistics.touch();

            Table::ObjectStatistics.put(
                tx,
                self.hash,
                bincode::serialize(&statistics).expect("can serialize"),
            );
        }

        Ok(())
    }

    /// Tries to resolve a content riddle against all objects currently in the database.
    pub fn find<Tx: TxHandle>(
        tx: &Tx,
        content_riddle: &Riddle,
        hint: &Hint,
    ) -> Result<Option<ObjectRef>, crate::Error> {
        let outcome = Table::Objects.prefix(hint.prefix()).for_each(tx, |key, _| {
            let hash: Hash = match key.try_into() {
                Ok(hash) => hash,
                Err(err) => {
                    tracing::warn!("{}", err);
                    return None;
                }
            };

            if content_riddle.resolves(&hash) {
                return Some(ObjectRef { hash });
            }

            None
        });

        Ok(outcome)
    }

    /// Creates an object in the database.
    fn create_object_with(
        tx: &mut WritableTx,
        hash: Hash,
        metadata: &ObjectMetadata,
        statistics: &ObjectStatistics,
        bookmark: bool,
    ) {
        // Do not insert if object already exists. This will overwrite information!
        if ObjectRef::new(hash).exists(tx).unwrap_or(false) {
            tracing::info!("Object {hash} already exists in the database; skipping creation");
            return;
        }
        Table::Objects.put(tx, hash, []);
        Table::ObjectMetadata.put(
            tx,
            hash,
            bincode::serialize(&metadata).expect("can serialize"),
        );
        Table::ObjectStatistics.put(
            tx,
            hash,
            bincode::serialize(&statistics).expect("can serialize"),
        );

        for chunk_hash in &metadata.hashes {
            Table::ObjectChunkRefCount.map(tx, chunk_hash, MergeOperation::Increment(1).merger());
        }

        if bookmark {
            Bookmark::new(BookmarkType::User, ObjectRef { hash }).mark(tx);
        }
    }

    /// Build a new object from data coming from a _trusted_ source.
    pub fn build(
        header: ObjectHeader,
        bookmark: bool,
        source: impl 'static + Send + IntoIterator<Item = Result<u8, crate::Error>>,
    ) -> Result<ObjectRef, crate::Error> {
        let mut content_size = 0;
        let mut buffer = header.buffer(); // start the first chunk with the serialized header
        let mut hashes = Vec::new();
        let mut source = source.into_iter();

        loop {
            // Extend buffer until (a) source stops (b) error (c) reaches limit.
            for byte in &mut source {
                buffer.push(byte?);

                if buffer.len() == CHUNK_SIZE {
                    break;
                }
            }

            content_size += buffer.len();

            let chunk_hash = Hash::from_bytes(&buffer);

            writable_tx(|tx| {
                Table::ObjectChunks.put(tx, chunk_hash.as_ref(), buffer.as_slice());
                Ok(())
            })
            .expect("infalible");

            hashes.push(chunk_hash);

            // Buffer not fille to the brim: it's over!
            if buffer.len() < CHUNK_SIZE {
                break;
            }

            // Else clean buffer!
            buffer.clear();
        }

        let merkle_tree = MerkleTree::from(hashes);
        let hash = merkle_tree.root();
        let metadata = ObjectMetadata {
            hashes: merkle_tree.hashes().to_vec(),
            header,
            content_size,
            received_at: chrono::Utc::now(),
        };
        let statistics = ObjectStatistics::new(content_size, Duration::from_secs(0));

        tracing::info!(
            "New object {hash} of type {} and size {:.3}kB",
            metadata.header.content_type,
            metadata.content_size as f64 / 1_000.0,
        );
        tracing::debug!("Object {hash} metadata is {:#?}", metadata);

        writable_tx(|tx| {
            ObjectRef::create_object_with(tx, hash, &metadata, &statistics, bookmark);
            Ok(())
        })
        .unwrap();

        Ok(ObjectRef { hash })
    }

    /// Imports an existing object in the database from an external _already validated_ data source,
    /// returning a _ContentStream_ to the incoming validated bytes.
    pub fn import(
        merkle_tree: MerkleTree,
        supplied_metadata: ObjectMetadata,
        query_duration: Duration,
        chunks: impl 'static + Send + Unpin + Stream<Item = Result<Vec<u8>, crate::Error>>,
    ) -> ContentStream {
        let (send, recv) = mpsc::unbounded_channel();
        let task_send = send.clone();
        let mut next_to_send = 0usize;
        let mut arrived_chunks = BTreeMap::new();

        // Spawn importing task
        tokio::spawn(async move {
            if let Err(err) =
                ObjectRef::do_import(merkle_tree, supplied_metadata, query_duration, send, chunks)
                    .await
            {
                task_send.send(Err(err)).ok();
            }
        });

        // Create a stream that will stream the received data _from the database_ contiguously
        // (chunks may arrive out of order).
        let hashes = stream::try_unfold(recv, |mut recv| async move {
            let yielded = recv.recv().await.transpose()?;
            Ok(yielded.map(|y| (y, recv))) as Result<_, crate::Error>
        })
        .map_ok(move |(chunk_id, hash)| {
            arrived_chunks.insert(chunk_id, hash);
            let mut contiguous_hashes = vec![];

            while let Some(&hash) = arrived_chunks.get(&next_to_send) {
                contiguous_hashes.push(hash);
                next_to_send += 1;
            }

            stream::iter(contiguous_hashes.into_iter().map(Ok))
        })
        .try_flatten();

        ContentStream {
            hashes: Box::pin(hashes),
            is_error: false,
            skip_header: true,
        }
    }

    /// Imports an existing object in the database from an external _already validated_ data source.
    async fn do_import(
        merkle_tree: MerkleTree,
        supplied_metadata: ObjectMetadata,
        query_duration: Duration,
        sender: mpsc::UnboundedSender<Result<(usize, Hash), crate::Error>>,
        chunks: impl Unpin + Stream<Item = Result<Vec<u8>, crate::Error>>,
    ) -> Result<(), crate::Error> {
        // Having a map allows us to receive chunks out of order.
        let hash = merkle_tree.root();
        let hashes = Arc::new(
            merkle_tree
                .hashes()
                .iter()
                .copied()
                .enumerate()
                .map(|(chunk_id, chunk_hash)| (chunk_hash, chunk_id))
                .collect::<BTreeMap<_, _>>(),
        );

        // Start receiving chunks:
        let mut arrived_chunks = vec![false; merkle_tree.len()];
        let mut content_size = 0;
        let mut maybe_header = None;
        let mut limited_chunks = chunks.take(merkle_tree.len());

        while let Some(chunk) = limited_chunks.next().await.transpose()? {
            // Check if hash actually corresponds to hash in merkle tree.
            let received_hash = Hash::from_bytes(&chunk);
            let Some(chunk_id) = hashes.get(&received_hash).copied() else {
                return Err(format!(
                    "Received chunk has hash {received_hash}; which was not expected"
                )
                .into());
            };

            // Extract object header in the first chunk:
            if chunk_id == 0 {
                let (_, header) = ObjectHeader::read(chunk.iter().copied().map(Ok))?;

                if header != supplied_metadata.header {
                    return Err(format!(
                        "Supplied object header {:?} is not equal to transmitted header {:?}",
                        supplied_metadata.header, header
                    )
                    .into());
                }

                maybe_header = Some(header);
            }

            // Warn of incompatible chunk size (big chunks are dealt with somewhere else):
            if chunk.len() != CHUNK_SIZE && chunk_id != merkle_tree.len() - 1 {
                tracing::warn!(
                    "Expected standard size chunk, but got chunk of size {}kB. Incompatibly \
                    sized chunks might become illegal in the future.",
                    chunk.len() / 1_000
                );
            }

            // Put chunk in the database
            writable_tx(|tx| {
                Table::ObjectChunks.put(tx, received_hash.as_ref(), chunk.as_slice());
                Ok(())
            })
            .expect("infalible");

            // Emit received chunk:
            tracing::info!("Chunk {chunk_id} for object {hash} received");
            sender.send(Ok((chunk_id, received_hash))).ok();

            // Next chunk!
            content_size += chunk.len();
            arrived_chunks[chunk_id] = true;
        }

        // Check if _all_ chunks were ingested
        let not_arrived = arrived_chunks
            .into_iter()
            .enumerate()
            .filter(|&(_, x)| !x)
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if !not_arrived.is_empty() {
            return Err(format!(
                "Insufficient chunks for object {} received: missing {:?}",
                merkle_tree.root(),
                not_arrived,
            )
            .into());
        }

        // Build object:
        let metadata = ObjectMetadata {
            hashes: merkle_tree.hashes().to_vec(),
            header: maybe_header.ok_or(crate::Error::NoHeaderRead)?,
            content_size,
            received_at: chrono::Utc::now(),
        };
        let statistics = ObjectStatistics::new(content_size, query_duration);

        writable_tx(|tx| {
            ObjectRef::create_object_with(tx, hash, &metadata, &statistics, false);
            tracing::info!("New object {} with metadata: {:#?}", hash, metadata);

            Ok(())
        })
    }

    /// Create a copy of this object, but with a different nonce header value. This new object
    /// will have a new content hash.
    pub fn reissue(&self, bookmark: bool) -> Result<Option<ObjectRef>, crate::Error> {
        if let Some(mut iter) = self.iter_bytes(false)? {
            let (_, header) = ObjectHeader::read(&mut iter)?;
            let reissued = ObjectRef::build(header.reissue(), bookmark, iter)?;

            Ok(Some(reissued))
        } else {
            Ok(None)
        }
    }

    /// Iterates through the contents of an object, optionally including the header part
    /// if `skip_header` is set.
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    pub fn iter_bytes(&self, skip_header: bool) -> Result<Option<BytesIter>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = readonly_tx(|tx| self.metadata(tx))?
        {
            metadata
        } else {
            return Ok(None);
        };

        // Touched because a `BytesIter` is created.
        writable_tx(|tx| self.touch(tx))?;

        Ok(Some(BytesIter {
            hashes: metadata.hashes.into_iter(),
            current_chunk: None,
            is_error: false,
            skip_header,
        }))
    }

    /// Streams the contents of an object, optionally including the header part if `skip_header`
    /// is set.
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    pub fn stream_content(&self, skip_header: bool) -> Result<Option<ContentStream>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = readonly_tx(|tx| self.metadata(tx))?
        {
            metadata
        } else {
            return Ok(None);
        };

        // Touched because a `BytesIter` is created.
        writable_tx(|tx| self.touch(tx))?;

        Ok(Some(ContentStream {
            hashes: Box::pin(stream::iter(metadata.hashes.clone().into_iter().map(Ok))),
            is_error: false,
            skip_header,
        }))
    }

    /// Streams the contents of an object.
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    pub fn iter_content(&self) -> Result<Option<ContentIter>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = readonly_tx(|tx| self.metadata(tx))?
        {
            metadata
        } else {
            return Ok(None);
        };

        // Touched because a `BytesIter` is created.
        writable_tx(|tx| self.touch(tx))?;

        Ok(Some(ContentIter {
            hashes: metadata.hashes.into_iter(),
            is_error: false,
        }))
    }

    /// Returns the whole content of this object as a `Vec<u8>`.
    ///
    /// # Note
    ///
    /// Be careful when using this method. If the file is too big, you might get out of
    /// memory!
    pub fn content(&self) -> Result<Option<Vec<u8>>, crate::Error> {
        if let Some(iter) = self.iter_bytes(true)? {
            Ok(Some(iter.collect::<Result<Vec<_>, _>>()?))
        } else {
            Ok(None)
        }
    }

    /// Returns a bookmark handle for the supplied bookmark type (see [`BookmarkType`]).
    ///
    /// # Note
    ///
    /// Make sure that the object exists before marking objects, since the bookmark will leak
    /// space in the database if it doesn't.
    pub fn bookmark(&self, ty: BookmarkType) -> Bookmark {
        Bookmark::new(ty, self.clone())
    }

    /// Returns `Ok(true)` if this object is bookmarked by any [`BookmarkType`]. If the object
    /// does not exist in the database, this function returns `Ok(false)`. You need to further
    /// check if the object actually exists.
    pub fn is_bookmarked<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        let reference = Bookmark::new(BookmarkType::Reference, self.clone());
        let user = Bookmark::new(BookmarkType::User, self.clone());

        Ok(reference.is_marked(tx)? || user.is_marked(tx)?)
    }

    /// Returns `Ok(true)` if this is a draft object. If the object does not exist in the
    /// database, this function returns `Ok(true)`. You may need to further check if the object
    ///  actually exists.
    pub fn is_draft<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        Ok(self
            .metadata(tx)?
            .map(|m| m.header.is_draft)
            .unwrap_or(true))
    }

    /// Create a self-sealed object for this object. A self-sealed object is an object that is
    /// generated by the contents of another object, ciphered using its own hash. This allows the
    /// contents of this object to be shared with third parties, without the risk of leaking
    /// either the content or the hash of this object.
    pub fn self_seal(&self) -> Result<ObjectRef, crate::Error> {
        todo!()
    }
}

/// Statistics on object usage. This entity is used by the vacuum system to decide which objects
/// are due for automatic deletion due to lack of usage.
#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectStatistics {
    /// The content size of this object.
    size: usize,
    /// Time the object object was built or imported in this database.
    created_at: DateTime<Utc>,
    /// The last time somebody touched this object.
    last_touched_at: DateTime<Utc>,
    /// Total number of touches on this object.
    touches: usize,
    /// The time it took for the network to respond with a valid candidate for this object.
    #[serde(default)]
    query_duration: Duration,
}

/// The prior distribution parameters (_a priori_ suppositions) about object usage.
#[derive(Debug)]
pub struct UsePrior {
    pub gamma_alpha: f64,
    pub gamma_beta: f64,
    pub beta_alpha: f64,
    pub beta_beta: f64,
}

impl Default for UsePrior {
    fn default() -> UsePrior {
        UsePrior {
            gamma_alpha: 1.,
            gamma_beta: 86400., // one day in secs
            beta_alpha: 1.,
            beta_beta: 1.,
        }
    }
}

impl ObjectStatistics {
    /// Create a new statistics struct for an object of given size.
    fn new(size: usize, query_duration: Duration) -> ObjectStatistics {
        ObjectStatistics {
            size,
            created_at: Utc::now(),
            last_touched_at: Utc::now(),
            touches: 1,
            query_duration,
        }
    }

    /// Marks the object as touched in a specific point in time.
    pub fn touch(&mut self) {
        self.last_touched_at = Utc::now();
        self.touches += 1;
    }

    /// The size of the related object.
    pub fn size(&self) -> usize {
        self.size
    }

    /// This is a bit approximate modeling of the following process:
    /// a. First, the access pattern is a Poisson process of unknown rate. The prior is a
    ///    Gamma Distribution.
    /// b. After each touch, "toss a coin" to choose if you are still going to touch the
    ///    object ever again. This is a Bernoulli variable (coin toss) with unknown
    ///    probability. The prior is a Beta Distribution.
    pub fn byte_usefulness(&self, use_prior: &UsePrior) -> f64 {
        let time_inactive = (Utc::now() - self.last_touched_at).num_seconds() as f64;

        let post_gamma_alpha = use_prior.gamma_alpha + self.touches as f64;
        let post_gamma_beta =
            use_prior.gamma_beta + (self.last_touched_at - self.created_at).num_seconds() as f64;
        // One "pseudo observation" E[exp(-time_inactive * poisson_rate)].
        let survival_prob = (1. + time_inactive / post_gamma_beta).powf(-post_gamma_alpha);
        let post_beta_alpha = use_prior.beta_alpha + (1. - survival_prob);
        let post_beta_beta = use_prior.beta_beta + self.touches as f64;

        // Based on an uninformed beta distribution.
        // TODO: uninformed -> bad idea! Learn from other objects
        let prob_future_use = post_beta_beta / (post_beta_alpha + post_beta_beta);

        // Probability it is still going to be used (Bayes'):
        let prob_use = prob_future_use * survival_prob
            / (prob_future_use * survival_prob + (1. - prob_future_use));
        let expected_access_freq = post_gamma_alpha / post_gamma_beta;

        // Add 8kB to symbolize "hidden overhead": metadata, statistics, items, etc...
        prob_use * expected_access_freq / (self.size + 8_192) as f64
    }
}
