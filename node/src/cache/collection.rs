use samizdat_common::{Hash, PatriciaMap, PatriciaProof};
use serde_derive::{Deserialize, Serialize};

use crate::db;
use crate::Table;

use super::ObjectRef;

#[derive(Serialize, Deserialize)]
struct Locator<'a> {
    collection: CollectionRef,
    name: &'a str,
}

#[derive(Serialize, Deserialize)]
pub struct CollectionItem {
    pub object: ObjectRef,
    pub inclusion_proof: PatriciaProof,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRef {
    pub hash: Hash,
}

impl CollectionRef {
    pub fn new(hash: Hash) -> CollectionRef {
        CollectionRef { hash }
    }

    pub fn build<I, N>(objects: I) -> Result<CollectionRef, crate::Error>
    where
        I: AsRef<[(N, ObjectRef)]>,
        N: AsRef<str>,
    {
        // Note: this is the slow part of the process (by a long stretch)
        let patricia_map = objects
            .as_ref()
            .iter()
            .map(|(name, object)| (Hash::build(name.as_ref().as_bytes()), object.hash))
            .collect::<PatriciaMap>();

        let root = *patricia_map.root();
        let collection = CollectionRef { hash: root };

        let mut batch = rocksdb::WriteBatch::default();

        for (name, object) in objects.as_ref() {
            let key = collection.locator_for(name.as_ref());
            let value = CollectionItem {
                object: object.clone(),
                inclusion_proof: patricia_map
                    .proof_for(Hash::build(name.as_ref().as_bytes()))
                    .expect("name exists in map"),
            };
            dbg!(name.as_ref());
            batch.put_cf(
                Table::CollectionItems.get(),
                bincode::serialize(&key).expect("can serialize"),
                dbg!(bincode::serialize(&value).expect("can serialize")),
            );
        }

        batch.put_cf(Table::Collections.get(), collection.hash, &[]);

        db().write(batch)?;

        Ok(collection)
    }

    fn locator_for<'a>(&self, name: &'a str) -> Locator<'a> {
        Locator {
            collection: self.clone(),
            name,
        }
    }

    pub fn get(&self, name: &str) -> Result<Option<CollectionItem>, crate::Error> {
        let locator = self.locator_for(name);
        let maybe_item = db().get_cf(
            Table::CollectionItems.get(),
            bincode::serialize(&locator).expect("can serialize"),
        )?;
        dbg!(&maybe_item);

        if let Some(item) = maybe_item {
            Ok(Some(bincode::deserialize(&item)?))
        } else {
            Ok(None)
        }
    }
}
