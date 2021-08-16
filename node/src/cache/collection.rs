use rocksdb::IteratorMode;
use serde_derive::{Deserialize, Serialize};
use std::convert::TryInto;
use std::fmt::{self, Display};

use samizdat_common::{ContentRiddle, Hash, PatriciaMap, PatriciaProof};

use crate::db;
use crate::Table;

use super::ObjectRef;

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionItem {
    pub collection: CollectionRef,
    pub name: String,
    pub inclusion_proof: PatriciaProof,
}

impl CollectionItem {
    pub fn is_valid(&self) -> bool {
        let is_included = self.inclusion_proof.is_in(&self.collection.hash);
        let key = Hash::build(self.name.as_bytes());

        if !is_included {
            log::error!("Inclusion proof falied for {:?}", self);
            return false;
        }

        if &key != self.inclusion_proof.claimed_key() {
            log::error!("Key is different from claimed key: {:?}", self);
            return false;
        }

        true
    }

    /// Returns an object reference if item is valid. Else, returns
    /// `Ok(Error::InvalidCollectionItem)`.
    pub fn object(&self) -> Result<ObjectRef, crate::Error> {
        if self.is_valid() {
            Ok(ObjectRef::new(*self.inclusion_proof.claimed_value()))
        } else {
            Err(crate::Error::InvalidCollectionItem)
        }
    }

    pub fn locator(&self) -> Locator {
        Locator {
            collection: self.collection.clone(),
            name: &self.name,
        }
    }

    pub fn find(content_riddle: &ContentRiddle) -> Result<Option<CollectionItem>, crate::Error> {
        let iter = db().iterator_cf(Table::CollectionItems.get(), IteratorMode::Start);

        for (key, value) in iter {
            let hash: Hash = match key.as_ref().try_into() {
                Ok(hash) => hash,
                Err(err) => {
                    log::warn!("{}", err);
                    continue;
                }
            };

            if content_riddle.resolves(&hash) {
                return Ok(Some(bincode::deserialize(&*value)?));
            }
        }

        Ok(None)
    }

    pub fn insert(&self) -> Result<(), crate::Error> {
        let key = self.collection.locator_for(&self.name).hash();
        db().put_cf(
            Table::CollectionItems.get(),
            key,
            bincode::serialize(self).expect("can serialize"),
        )?;

        Ok(())
    }
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

        for (name, _object) in objects.as_ref() {
            let key = collection.locator_for(name.as_ref()).hash();
            let value = CollectionItem {
                collection: collection.clone(),
                name: name.as_ref().to_owned(),
                inclusion_proof: patricia_map
                    .proof_for(Hash::build(name.as_ref().as_bytes()))
                    .expect("name exists in map"),
            };
            batch.put_cf(
                Table::CollectionItems.get(),
                key,
                bincode::serialize(&value).expect("can serialize"),
            );
        }

        batch.put_cf(Table::Collections.get(), collection.hash, &[]);

        db().write(batch)?;

        Ok(collection)
    }

    pub fn locator_for<'a>(&self, name: &'a str) -> Locator<'a> {
        Locator {
            collection: self.clone(),
            name,
        }
    }

    pub fn get(&self, name: &str) -> Result<Option<CollectionItem>, crate::Error> {
        let locator = self.locator_for(name);
        let maybe_item = db().get_cf(Table::CollectionItems.get(), locator.hash())?;

        if let Some(item) = maybe_item {
            Ok(Some(bincode::deserialize(&item)?))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct Locator<'a> {
    collection: CollectionRef,
    name: &'a str,
}

impl<'a> Display for Locator<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.collection.hash, self.name)
    }
}

impl<'a> Locator<'a> {
    pub fn hash(&self) -> Hash {
        self.collection.hash.rehash(&Hash::build(self.name))
    }

    pub fn get(&self) -> Result<Option<CollectionItem>, crate::Error> {
        self.collection.get(self.name)
    }

    // pub fn with_proof(&self, inclusion_proof: PatriciaProof) -> CollectionItem {
    //     CollectionItem {
    //         collection: self.collection.clone(),
    //         name: self.name.to_owned(),
    //         inclusion_proof,
    //     }
    // }
}
