use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::borrow::Cow;
use std::convert::TryInto;
use std::fmt::{self, Display};
use std::str::FromStr;

use samizdat_common::{ContentRiddle, Hash, PatriciaMap, PatriciaProof};

use crate::db::{db, Table};

use super::{Dropable, ObjectRef};

/// The function transforming an arbitrary string into its canonical path form.
fn normalize(name: &str) -> Cow<str> {
    if name.ends_with('/') || name.starts_with('/') || name.contains("//") {
        let restructured = name
            .split('/')
            .filter(|slice| !slice.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        Cow::from(restructured)
    } else {
        Cow::from(name)
    }
}

/// An owned item path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemPathBuf(Box<str>);

impl<'a> From<&'a str> for ItemPathBuf {
    fn from(name: &'a str) -> ItemPathBuf {
        ItemPathBuf(normalize(name).into())
    }
}

impl From<String> for ItemPathBuf {
    fn from(name: String) -> ItemPathBuf {
        ItemPathBuf(normalize(&name).into())
    }
}

impl From<Box<str>> for ItemPathBuf {
    fn from(name: Box<str>) -> ItemPathBuf {
        ItemPathBuf(normalize(&name).into())
    }
}

impl FromStr for ItemPathBuf {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<ItemPathBuf, crate::Error> {
        Ok(s.into())
    }
}

impl ItemPathBuf {
    /// Transformes into a borrowed item path.
    fn as_path<'a>(&'a self) -> ItemPath<'a> {
        ItemPath(self.0.as_ref().into())
    }
}

/// A borrowed item path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemPath<'a>(Cow<'a, str>);

impl<'a> From<&'a str> for ItemPath<'a> {
    fn from(name: &'a str) -> ItemPath<'a> {
        ItemPath(normalize(name))
    }
}

impl<'a> ItemPath<'a> {
    /// Retrieves the string representation of this path, in its canonical form.
    pub fn as_str<'b: 'a>(&'a self) -> &'a str {
        &self.0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionItem {
    pub collection: CollectionRef,
    pub name: ItemPathBuf,
    pub inclusion_proof: PatriciaProof,
}

impl Dropable for CollectionItem {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        let path = self.name.as_path();
        let locator = self.collection.locator_for(path);

        log::info!("Removing item {}: {:#?}", locator, self);

        batch.delete_cf(Table::CollectionItemLocators.get(), locator.path());
        batch.delete_cf(Table::CollectionItems.get(), locator.hash());

        Ok(())
    }
}

impl CollectionItem {
    pub fn is_valid(&self) -> bool {
        let is_included = self.inclusion_proof.is_in(&self.collection.hash);
        let key = Hash::build(self.name.0.as_bytes());

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

    pub fn locator<'a>(&'a self) -> Locator<'a> {
        Locator {
            collection: self.collection.clone(),
            name: self.name.as_path(),
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

    pub fn insert_with(&self, batch: &mut WriteBatch) {
        let locator = self.collection.locator_for(self.name.as_path());

        batch.put_cf(
            Table::CollectionItems.get(),
            locator.hash(),
            bincode::serialize(self).expect("can serialize"),
        );
        batch.put_cf(
            Table::CollectionItemLocators.get(),
            locator.path(),
            locator.hash(),
        );

        log::info!("Inserting item {}: {:#?}", locator, self);
    }

    pub fn insert(&self) -> Result<(), crate::Error> {
        let mut batch = WriteBatch::default();
        self.insert_with(&mut batch);
        db().write(batch)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRef {
    pub hash: Hash,
}

impl Dropable for CollectionRef {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        for name in self.list() {
            if let Some(item) = self.get(name.as_path())? {
                item.drop_if_exists_with(batch)?;
            }
        }

        Ok(())
    }
}

impl CollectionRef {
    pub fn new(hash: Hash) -> CollectionRef {
        CollectionRef { hash }
    }

    #[cfg(test)]
    pub(crate) fn rand() -> CollectionRef {
        CollectionRef { hash: Hash::rand() }
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
            .map(|(name, object)| (Hash::build(name.as_ref().as_bytes()), *object.hash()))
            .collect::<PatriciaMap>();

        let root = *patricia_map.root();
        let collection = CollectionRef { hash: root };

        let mut batch = WriteBatch::default();

        for (name, _object) in objects.as_ref() {
            let item = CollectionItem {
                collection: collection.clone(),
                name: name.as_ref().into(),
                inclusion_proof: patricia_map
                    .proof_for(Hash::build(name.as_ref().as_bytes()))
                    .expect("name exists in map"),
            };

            item.insert_with(&mut batch);
        }

        // batch.put_cf(Table::Collections.get(), collection.hash, &[]);

        db().write(batch)?;

        Ok(collection)
    }

    pub fn locator_for<'a>(&self, name: ItemPath<'a>) -> Locator<'a> {
        Locator {
            collection: self.clone(),
            name,
        }
    }

    pub fn get(&self, name: ItemPath) -> Result<Option<CollectionItem>, crate::Error> {
        let locator = self.locator_for(name);
        let maybe_item = db().get_cf(Table::CollectionItems.get(), locator.hash())?;

        if let Some(item) = maybe_item {
            Ok(Some(bincode::deserialize(&item)?))
        } else {
            Ok(None)
        }
    }

    pub fn list<'a>(&'a self) -> impl 'a + Iterator<Item = ItemPathBuf> {
        db().prefix_iterator_cf(Table::CollectionItemLocators.get(), self.hash.as_ref())
            .map(move |(key, _)| {
                ItemPathBuf::from(&*String::from_utf8_lossy(&key[self.hash.as_ref().len()..]))
            })
    }

    pub fn list_objects<'a>(
        &'a self,
    ) -> impl 'a + Iterator<Item = Result<ObjectRef, crate::Error>> {
        self.list().filter_map(move |name| {
            let locator = Locator {
                collection: self.clone(),
                name: name.as_path(),
            };
            locator.get_object().transpose()
        })
    }
}

#[derive(Debug)]
pub struct Locator<'a> {
    collection: CollectionRef,
    name: ItemPath<'a>,
}

impl<'a> Display for Locator<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.collection.hash, self.name.0)
    }
}

impl<'a> Locator<'a> {
    pub fn hash(&self) -> Hash {
        self.collection
            .hash
            .rehash(&Hash::build(self.name.0.as_ref()))
    }

    pub fn path(&self) -> Vec<u8> {
        [self.collection.hash.as_ref(), self.name.0.as_bytes()].concat()
    }

    pub fn get(&self) -> Result<Option<CollectionItem>, crate::Error> {
        self.collection.get(self.name.clone())
    }

    pub fn get_object(&self) -> Result<Option<ObjectRef>, crate::Error> {
        if let Some(item) = self.get()? {
            Ok(Some(item.object()?))
        } else {
            Ok(None)
        }
    }
}
