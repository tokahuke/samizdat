use futures::prelude::*;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::convert::TryInto;

use samizdat_common::{ContentRiddle, Hash, MerkleTree};

use crate::db::{db, Table};

use super::{Bookmark, BookmarkType, Dropable};

pub const CHUNK_SIZE: usize = 256_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub content_type: String,
    pub content_size: usize,
    pub hashes: Vec<Hash>,
}

pub struct ObjectStream {
    pub iter_chunks: Box<dyn Send + Unpin + Iterator<Item = Result<Vec<u8>, crate::Error>>>,
}

impl IntoIterator for ObjectStream {
    type Item = Result<Vec<u8>, crate::Error>;
    type IntoIter = Box<dyn Send + Unpin + Iterator<Item = Result<Vec<u8>, crate::Error>>>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_chunks
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
    /// Creates a new object refrerence from a hash.
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
    /// vacuum daemon that this object is usefull and therefore a worse candidate for deletion.
    /// 
    /// This fucntion has no effect if the object does not exist.
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

    /// Creates a new object in the database from external data.
    pub async fn build(
        content_type: String,
        expected_content_size: usize,
        source: impl Unpin + Stream<Item = Result<u8, crate::Error>>,
    ) -> Result<(ObjectMetadata, ObjectRef), crate::Error> {
        let mut content_size = 0;
        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut hashes = Vec::new();

        let mut limited_source = source.take(expected_content_size);

        loop {
            while let Some(byte) = limited_source.next().await {
                buffer.push(byte?);
                content_size += 1;

                if buffer.len() == CHUNK_SIZE {
                    break;
                }
            }

            let chunk_hash = Hash::build(&buffer);
            db().put_cf(
                Table::ObjectChunks.get(),
                bincode::serialize(&chunk_hash).expect("can serialize"),
                &buffer,
            )?;
            hashes.push(chunk_hash);

            if content_size == expected_content_size {
                break;
            }

            if buffer.len() < CHUNK_SIZE {
                break;
            }

            buffer.clear();
        }

        let merkle_tree = MerkleTree::from(hashes);
        let hash = merkle_tree.root();
        let metadata = ObjectMetadata {
            content_type,
            content_size,
            hashes: merkle_tree.hashes().to_vec(),
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

        db().write(batch)?;

        Ok((metadata, ObjectRef { hash }))
    }

    /// Streams the contents of an object.
    /// 
    /// This function returns `Ok(None)` if the object does not actually exist.
    ///  
    /// TODO: lock for reading? Reading is not atomic. (snapshots?)
    pub fn iter(&self) -> Result<Option<ObjectStream>, crate::Error> {
        let metadata: ObjectMetadata = if let Some(metadata) = self.metadata()? {
            metadata
        } else {
            return Ok(None);
        };

        // Not as efficient as iterating, but large chunk => don't matter.
        let iter_chunks = metadata.hashes.clone().into_iter().map(|hash| {
            let chunk = db()
                .get_cf(Table::ObjectChunks.get(), &hash)?
                .ok_or_else(|| format!("Chunk missing: {}", hash))?;
            Ok(chunk)
        });

        Ok(Some(ObjectStream {
            iter_chunks: Box::new(iter_chunks),
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
}

use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectStatistics {
    size: usize,
    created_at: DateTime<Utc>,
    last_touched_at: DateTime<Utc>,
    touches: usize,
}

impl ObjectStatistics {
    fn new(size: usize) -> ObjectStatistics {
        ObjectStatistics {
            size,
            created_at: Utc::now(),
            last_touched_at: Utc::now(),
            touches: 1,
        }
    }

    pub fn touch(&mut self) {
        self.last_touched_at = Utc::now();
        self.touches += 1;
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// This is a bit approximate modeling of the following process:
    /// a. First, the access pattern is a Poisson process.
    /// b. After each touch, "toss a coin" to choose if you are still going to touch the object
    /// ever again.
    /// TODO: needs more rigorous implementation. This makes some (gross?) mathematical
    /// simplifications.
    pub fn byte_usefulness(&self) -> f64 {
        // Add one day as a prior.
        let total_time = (self.last_touched_at - self.created_at).num_seconds() as f64 + 86400.;
        let access_freq = self.touches as f64 / total_time;
        let time_inactive = (Utc::now() - self.last_touched_at).num_seconds() as f64;

        // Based on an uninformed beta distribution.
        // TODO: uninformed -> bad idea! Learn from other objects
        let prob_future_use = self.touches as f64 / (1. + self.touches as f64);

        // Probability it is still going to be used (Bayes'):
        let survival =
            (1. + time_inactive / access_freq / self.touches as f64).powi(-(self.touches as i32));
        let prob_use =
            prob_future_use * survival / (prob_future_use * survival + (1. - prob_future_use));

        // Add 8kB to symbolize "hidden overhead": metadata, statstics, items, etc...
        prob_use * access_freq / (self.size + 8_192) as f64
    }
}
