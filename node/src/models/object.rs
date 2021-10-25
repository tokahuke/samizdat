use chrono::{DateTime, Utc};
use futures::prelude::*;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::convert::TryInto;

use samizdat_common::{ContentRiddle, Hash, MerkleTree};

use crate::db::{db, Table};

use super::{Bookmark, BookmarkType, Dropable};

/// The size of a chunk. An object consists of a sequence of chunks, the hash
/// of which are used to create the Merkle tree whose root hash is the object
pub const CHUNK_SIZE: usize = 256_000;

/// The first section before the actual content of the object. The header is
/// encoded as a null-escaped byte sequence in the beginning of the first chunk.
#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectHeader {
    /// The MIME type of this object.
    pub content_type: String,
    /// Whether this is a draft object or not. Draft objects cannot be shared
    /// publicly.
    pub is_draft: bool,
    /// The date of creation of this object.
    pub created_at: DateTime<Utc>,
    /// A number with no semantics whatsoever. You can use this to create a
    /// different object hash for the same content.
    pub nonce: u64,
}

impl ObjectHeader {
    /// Reads a header from an iterator of bytes.
    ///
    /// TODO: should be `Read` instead of `Iterator`?
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
fn get_chunk(hash: Hash) -> Result<Vec<u8>, crate::Error> {
    Ok(db()
        .get_cf(Table::ObjectChunks.get(), &hash)?
        .ok_or_else(|| format!("Chunk missing: {}", hash))?)
}

/// Information about the object that is "out of band", that is, does not compose the hash
/// directly. This is used for internal bookkeeping inside the node.
#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectMetadata {
    /// The hashes of each chunk in the order that they appear.
    pub hashes: Vec<Hash>,
    /// This field is informational and for convenience only. The _real_ header is in the
    /// first bytes of the first chunk.
    pub header: ObjectHeader,
    /// Sum of the sizes of all chunks. This includes the header size.
    pub content_size: usize,
}

/// An iterator over the bytes of an object, including its header.
pub struct ContentIter {
    /// An iterator over hashes.
    hashes: std::vec::IntoIter<Hash>,
    /// An iterator voer the current chunk.
    current_chunk: Option<std::vec::IntoIter<u8>>,
    /// Indicates whether an error has occurred.
    is_error: bool,
}

impl Iterator for ContentIter {
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
            match get_chunk(hash) {
                // Found chunk? Load an try again!
                Ok(chunk) => {
                    self.current_chunk = Some(chunk.into_iter());
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
pub struct ChunkIter {
    /// An iterator over hashes.
    hashes: std::vec::IntoIter<Hash>,
    /// Indicates whether an error has occurred.
    is_error: bool,
}

impl Iterator for ChunkIter {
    type Item = Result<Vec<u8>, crate::Error>;
    fn next(&mut self) -> Option<Result<Vec<u8>, crate::Error>> {
        // Fused on error:
        if self.is_error {
            return None;
        }

        // Try get new chunk:
        if let Some(hash) = self.hashes.next() {
            match get_chunk(hash) {
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

/// A handle to an object. The object does not necessarily needs to exist in the database.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectRef {
    /// The hash that defines this object.
    hash: Hash,
}

impl Dropable for ObjectRef {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        log::info!("Removing object {:?}", self);

        let metadata: ObjectMetadata = match db().get_cf(Table::ObjectMetadata.get(), &self.hash)? {
            Some(serialized) => bincode::deserialize(&serialized)?,
            None => return Ok(()),
        };

        for hash in &metadata.hashes {
            batch.delete_cf(Table::ObjectChunks.get(), hash);
        }

        self.bookmark(BookmarkType::Reference).clear_with(batch);
        self.bookmark(BookmarkType::User).clear_with(batch);

        batch.delete_cf(Table::ObjectStatistics.get(), &self.hash);
        batch.delete_cf(Table::ObjectMetadata.get(), &self.hash);
        batch.delete_cf(Table::Objects.get(), &self.hash);

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

    /// Returns the metadata on this object. This function returns `Ok(None)` if the object
    /// does not actually exist.
    pub fn metadata(&self) -> Result<Option<ObjectMetadata>, crate::Error> {
        match db().get_cf(Table::ObjectMetadata.get(), &self.hash)? {
            Some(serialized) => Ok(Some(bincode::deserialize(&serialized)?)),
            None => Ok(None),
        }
    }

    /// Update statistics indicating that this object was used. This will signal to the
    /// vacuum daemon that this object is useful and therefore a worse candidate for deletion.
    ///
    /// This function has no effect if the object does not exist.
    ///
    /// TODO: current impl allows for TOCTOU.
    pub fn touch(&self) -> Result<(), crate::Error> {
        if let Some(statistics) = db().get_cf(Table::ObjectStatistics.get(), self.hash)? {
            let mut statistics: ObjectStatistics = bincode::deserialize(&statistics)?;
            statistics.touch();
            db().put_cf(
                Table::ObjectStatistics.get(),
                self.hash,
                bincode::serialize(&statistics).expect("can serialize"),
            )?;
        }

        Ok(())
    }

    /// Tries to resolve a content riddle against all objects currently in the database.
    pub fn find(content_riddle: &ContentRiddle) -> Option<ObjectRef> {
        let iter = db().iterator_cf(Table::Objects.get(), IteratorMode::Start);

        for (key, _) in iter {
            let hash: Hash = match key.as_ref().try_into() {
                Ok(hash) => hash,
                Err(err) => {
                    log::warn!("{}", err);
                    continue;
                }
            };

            if content_riddle.resolves(&hash) {
                return Some(ObjectRef { hash });
            }
        }

        None
    }

    /// Build a new object from data coming from a _trusted_ source.
    pub fn build(
        header: ObjectHeader,
        bookmark: bool,
        source: impl IntoIterator<Item = Result<u8, crate::Error>>,
    ) -> Result<ObjectRef, crate::Error> {
        let mut content_size = 0;
        let mut buffer = header.buffer(); // start the first chunk with the serialized eader
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

            let chunk_hash = Hash::build(&buffer);
            db().put_cf(Table::ObjectChunks.get(), &chunk_hash, &buffer)?;
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
        };
        let statistics = ObjectStatistics::new(content_size);

        log::info!("New object {} with metadata: {:#?}", hash, metadata);

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(Table::Objects.get(), &hash, &[]);
        batch.put_cf(
            Table::ObjectMetadata.get(),
            &hash,
            bincode::serialize(&metadata).expect("can serialize"),
        );
        batch.put_cf(
            Table::ObjectStatistics.get(),
            &hash,
            bincode::serialize(&statistics).expect("can serialize"),
        );

        if bookmark {
            Bookmark::new(BookmarkType::User, ObjectRef { hash }).mark_with(&mut batch);
        }

        db().write(batch)?;

        Ok(ObjectRef { hash })
    }

    /// Imports an existing object in the database from an external data.
    pub async fn import(
        expected_content_size: usize,
        bookmark: bool,
        source: impl Unpin + Stream<Item = Result<u8, crate::Error>>,
    ) -> Result<ObjectRef, crate::Error> {
        let mut content_size = 0;
        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut hashes = Vec::new();
        let mut maybe_header = None;

        let mut limited_source = source.take(expected_content_size);

        loop {
            // Extend buffer until (a) source stops (b) error (c) reaches limit.
            while let Some(byte) = limited_source.next().await {
                buffer.push(byte?);
                content_size += 1;

                if buffer.len() == CHUNK_SIZE {
                    break;
                }
            }

            let chunk_hash = Hash::build(&buffer);
            db().put_cf(Table::ObjectChunks.get(), &chunk_hash, &buffer)?;
            hashes.push(chunk_hash);

            if maybe_header.is_none() {
                let (_read, header) =
                    ObjectHeader::read(buffer.iter().copied().map(Ok))?;
                maybe_header = Some(header);
            }

            // Buffer not fille to the brim: it's over!
            if buffer.len() < CHUNK_SIZE {
                break;
            }

            buffer.clear();
        }

        let merkle_tree = MerkleTree::from(hashes);
        let hash = merkle_tree.root();
        let metadata = ObjectMetadata {
            hashes: merkle_tree.hashes().to_vec(),
            header: maybe_header.ok_or(crate::Error::NoHeaderRead)?,
            content_size,
        };
        let statistics = ObjectStatistics::new(content_size);

        log::info!("New object {} with metadata: {:#?}", hash, metadata);

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(Table::Objects.get(), &hash, &[]);
        batch.put_cf(
            Table::ObjectMetadata.get(),
            &hash,
            bincode::serialize(&metadata).expect("can serialize"),
        );
        batch.put_cf(
            Table::ObjectStatistics.get(),
            &hash,
            bincode::serialize(&statistics).expect("can serialize"),
        );

        if bookmark {
            Bookmark::new(BookmarkType::User, ObjectRef { hash }).mark_with(&mut batch);
        }

        db().write(batch)?;

        Ok(ObjectRef { hash })
    }

    /// Create a copy of this object, but with a different nonce header value. This new object
    /// will have a new content hash.
    pub fn reissue(&self, bookmark: bool) -> Result<Option<ObjectRef>, crate::Error> {
        if let Some(mut iter) = self.iter()? {
            let (_, header) = ObjectHeader::read(&mut iter)?;
            let reissued = ObjectRef::build(
                ObjectHeader {
                    nonce: rand::random(),
                    ..header
                },
                bookmark,
                iter,
            )?;

            Ok(Some(reissued))
        } else {
            Ok(None)
        }
    }

    /// Streams the contents of an object, including the header part. To skip it, see
    /// [`ObjectRef::iter_skip_header`].
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    ///  
    /// TODO: lock for reading? Reading is not atomic. (snapshots?)
    pub fn iter(&self) -> Result<Option<ContentIter>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = self.metadata()? {
            metadata
        } else {
            return Ok(None);
        };

        Ok(Some(ContentIter {
            hashes: metadata.hashes.into_iter(),
            current_chunk: None,
            is_error: false,
        }))
    }

    /// Streams the contents of an object, skipping the header part.
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    ///  
    /// TODO: lock for reading? Reading is not atomic. (snapshots?)
    pub fn iter_skip_header(&self) -> Result<Option<ContentIter>, crate::Error> {
        if let Some(mut iter) = self.iter()? {
            let (_read, _header) = ObjectHeader::read(&mut iter)?;
            Ok(Some(iter))
        } else {
            Ok(None)
        }
    }

    /// Streams the contents of an object.
    ///
    /// This function returns `Ok(None)` if the object does not actually exist.
    ///  
    /// TODO: lock for reading? Reading is not atomic. (snapshots?)
    pub fn chunks(&self) -> Result<Option<ChunkIter>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = self.metadata()? {
            metadata
        } else {
            return Ok(None);
        };

        Ok(Some(ChunkIter {
            hashes: metadata.hashes.into_iter(),
            is_error: false,
        }))
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
    pub fn is_bookmarked(&self) -> Result<bool, crate::Error> {
        // TODO: where is iterator error?
        Ok(db()
            .prefix_iterator_cf(Table::Bookmarks.get(), self.hash)
            .next()
            .is_some())
    }

    /// Returns `Ok(true)` if this is a draft object. If the object does not exist in the
    /// database, this function returns `Ok(true)`. You may need to further check if the object
    ///  actually exists.
    pub fn is_draft(&self) -> Result<bool, crate::Error> {
        Ok(self.metadata()?.map(|m| m.header.is_draft).unwrap_or(true))
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
}

/// The prior distribution parameters (_a priori_ suppositions) about object usage.
#[derive(Debug)]
pub struct UsePrior {
    pub gamma_alpha: f64,
    pub gamma_beta: f64,
    pub beta_alpha: f64,
    pub beta_beta: f64,
}

impl ObjectStatistics {
    /// Create a new statistics struct for an object of given size.
    fn new(size: usize) -> ObjectStatistics {
        ObjectStatistics {
            size,
            created_at: Utc::now(),
            last_touched_at: Utc::now(),
            touches: 1,
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
        let post_beta_alpha = use_prior.beta_alpha + survival_prob;
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
