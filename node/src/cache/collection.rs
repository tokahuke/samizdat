use samizdat_common::{Hash, InclusionProof, MerkleTree};
use serde_derive::{Deserialize, Serialize};

use crate::db;
use crate::Table;

use super::ObjectRef;

struct Naming<'a>(&'a str, &'a ObjectRef);

impl<'a> Naming<'a> {
    fn hash(&self) -> Hash {
        Hash::build(self.0.as_bytes()).rehash(&self.1.hash)
    }
}

#[derive(Serialize, Deserialize)]
struct Locator {
    collection: CollectionRef,
    name: String,
}

impl Locator {
    pub fn hash(&self) -> Hash {
        self.collection.hash.rehash(&Hash::build(&self.name))
    }
}

#[derive(Serialize, Deserialize)]
pub struct CollectionItem {
    pub object: ObjectRef,
    pub inclusion_proof: InclusionProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRef {
    pub hash: Hash,
}

impl CollectionRef {
    pub fn new(hash: Hash) -> CollectionRef {
        CollectionRef { hash }
    }

    pub fn build<I, N>(it: I) -> Result<CollectionRef, crate::Error>
    where
        I: AsRef<[(N, ObjectRef)]>,
        N: AsRef<str>,
    {
        let hashes = it
            .as_ref()
            .iter()
            .map(|(name, object)| Naming(name.as_ref(), object).hash())
            .collect::<Vec<_>>();

        let merkle_tree = MerkleTree::from(hashes);
        let root = merkle_tree.root();
        let collection = CollectionRef { hash: root };

        let mut batch = rocksdb::WriteBatch::default();

        for (idx, (name, object)) in it.as_ref().iter().enumerate() {
            let key = collection.locator_for(name.as_ref().to_owned());
            let value = CollectionItem {
                object: object.clone(),
                inclusion_proof: merkle_tree.proof_for(idx).expect("index exists in list"),
            };

            batch.put_cf(
                Table::CollectionItems.get(),
                bincode::serialize(&key).expect("can serialize"),
                bincode::serialize(&value).expect("can serialize"),
            );
        }

        batch.put_cf(Table::Collections.get(), collection.hash, &[]);

        db().write(batch)?;

        Ok(collection)
    }

    fn locator_for(&self, name: String) -> Locator {
        Locator {
            collection: self.clone(),
            name,
        }
    }

    pub fn get(&self, name: String) -> Result<Option<CollectionItem>, crate::Error> {
        let locator = self.locator_for(name);
        let maybe_item = db().get_cf(
            Table::CollectionItems.get(),
            bincode::serialize(&locator).expect("can serialize"),
        )?;

        if let Some(item) = maybe_item {
            Ok(bincode::deserialize(&item)?)
        } else {
            Ok(None)
        }
    }
}
