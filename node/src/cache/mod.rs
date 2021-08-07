use futures::prelude::*;
use rocksdb::IteratorMode;
use serde_derive::{Deserialize, Serialize};
use std::convert::TryInto;

use samizdat_common::{ContentRiddle, Hash, InclusionProof, MerkleTree};

use crate::db;
use crate::db::Table;

pub const CHUNK_SIZE: usize = 256_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChunkMetadata {
    inclusion_proof: InclusionProof,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectMetadata {
    pub content_type: String,
    pub content_size: usize,
    pub hashes: Vec<Hash>,
}

pub struct ObjectStream {
    pub metadata: ObjectMetadata,
    pub iter_chunks: Box<dyn Send + Unpin + Iterator<Item = Result<Vec<u8>, crate::Error>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRef {
    pub hash: Hash,
}

impl ObjectRef {
    pub fn new(hash: Hash) -> ObjectRef {
        ObjectRef { hash }
    }

    pub fn find(content_riddle: &ContentRiddle) -> Option<ObjectRef> {
        let iter = db().iterator_cf(Table::Roots.get(), IteratorMode::Start);

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

    pub async fn build(
        content_type: String,
        expected_content_size: usize,
        mut source: impl Unpin + Stream<Item = Result<u8, crate::Error>>,
    ) -> Result<(ObjectMetadata, ObjectRef), crate::Error> {
        log::debug!("building new object");

        let mut content_size = 0;
        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut hashes = Vec::new();

        loop {
            while let Some(byte) = source.next().await {
                buffer.push(byte?);
                content_size += 1;

                if content_size == expected_content_size {
                    break;
                }

                if buffer.len() == CHUNK_SIZE {
                    break;
                }
            }

            let chunk_hash = Hash::build(&buffer);
            db().put_cf(
                Table::Chunks.get(),
                bincode::serialize(&chunk_hash).expect("can serialize"),
                &buffer,
            )?;
            hashes.push(chunk_hash);
            log::debug!("created chunk {}", chunk_hash);

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

        log::info!("New object {} with metadata: {:#?}", hash, metadata);

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(Table::Roots.get(), &hash, &[]);
        batch.put_cf(
            Table::Metadata.get(),
            &hash,
            bincode::serialize(&metadata).expect("can serialize"),
        );
        db().write(batch)?;

        Ok((metadata, ObjectRef { hash }))
    }

    pub fn drop_if_exists(self) -> Result<(), crate::Error> {
        let metadata: ObjectMetadata = match db().get_cf(Table::Metadata.get(), &self.hash)? {
            Some(serialized) => bincode::deserialize(&serialized)?,
            None => return Ok(()),
        };

        let mut batch = rocksdb::WriteBatch::default();
        for hash in &metadata.hashes {
            batch.delete_cf(Table::Chunks.get(), hash);
        }

        batch.delete_cf(Table::Metadata.get(), &self.hash);
        batch.delete_cf(Table::Roots.get(), &self.hash);

        db().write(batch)?;

        Ok(())
    }

    /// TODO: lock for reading. Reading is not atomic. (snapshots?)
    pub fn iter(&self) -> Result<Option<ObjectStream>, crate::Error> {
        let metadata: ObjectMetadata = match db().get_cf(Table::Metadata.get(), &self.hash)? {
            Some(serialized) => bincode::deserialize(&serialized)?,
            None => return Ok(None),
        };

        // Not as efficient as iterating, but large chunk => don't matter.
        let iter_chunks = metadata.hashes.clone().into_iter().map(|hash| {
            let chunk = db()
                .get_cf(Table::Chunks.get(), &hash)?
                .ok_or_else(|| format!("Chunk missing: {}", hash))?;
            Ok(chunk)
        });

        Ok(Some(ObjectStream {
            metadata,
            iter_chunks: Box::new(iter_chunks),
        }))
    }
}
