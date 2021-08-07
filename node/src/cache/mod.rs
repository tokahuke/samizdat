use futures::channel::mpsc;
use futures::prelude::*;

use samizdat_common::{Hash, MerkleTree};

use crate::db;
use crate::db::Table;

const CHUNK_SIZE: usize = 256_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRef {
    hash: Hash,
}

impl ObjectRef {
    pub async fn build(
        mut source: impl Unpin + Stream<Item = u8>,
    ) -> Result<ObjectRef, crate::Error> {
        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut hashes = Vec::new();

        loop {
            while let Some(byte) = source.next().await {
                buffer.push(byte);
                if buffer.len() == CHUNK_SIZE {
                    break;
                }
            }

            let chunk_hash = Hash::build(&buffer);
            let chunk_id = hashes.len();
            db().put_cf(
                Table::Chunks.get(),
                bincode::serialize(&chunk_hash).expect("can serialize"),
                &buffer,
            )?;
            hashes.push(chunk_hash);

            if buffer.len() < CHUNK_SIZE {
                break;
            }

            buffer.clear();
        }

        let merkle_tree = MerkleTree::from(hashes);
        let hash = merkle_tree.root();

        let mut batch = rocksdb::WriteBatch::default();
        batch.put_cf(Table::Roots.get(), &hash, &[]);
        batch.put_cf(
            Table::MerkleTrees.get(),
            &hash,
            bincode::serialize(merkle_tree.hashes()).expect("can serialize"),
        );
        db().write(batch)?;

        Ok(ObjectRef { hash })
    }

    pub fn drop_if_exists(self) -> Result<(), crate::Error> {
        let hashes: Vec<Hash> = match db().get_cf(Table::MerkleTrees.get(), &self.hash)? {
            Some(serialized) => bincode::deserialize(&serialized)?,
            None => return Ok(()),
        };

        let mut batch = rocksdb::WriteBatch::default();
        for hash in &hashes {
            batch.delete_cf(Table::Chunks.get(), hash);
        }

        batch.delete_cf(Table::MerkleTrees.get(), &self.hash);
        batch.delete_cf(Table::Roots.get(), &self.hash);

        db().write(batch)?;

        Ok(())
    }

    /// TODO: lock for reading. Reading is not atomic. (snapshots?)
    fn iter(
        &self,
    ) -> Result<Option<impl Iterator<Item = Result<Vec<u8>, crate::Error>>>, crate::Error> {
        let hashes: Vec<Hash> = match db().get_cf(Table::MerkleTrees.get(), &self.hash)? {
            Some(serialized) => bincode::deserialize(&serialized)?,
            None => return Ok(None),
        };

        // Not as efficient as iterating, but large chunk => don't matter.
        let iter_chunks = hashes.into_iter().map(|hash| {
            let chunk = db()
                .get_cf(Table::Chunks.get(), &hash)?
                .ok_or_else(|| format!("Chunk missing: {}", hash))?;
            Ok(chunk)
        });

        Ok(Some(iter_chunks))
    }
}
