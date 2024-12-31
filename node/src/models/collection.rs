//! Collections are a set of objects indexed by human-readable names. Collections are
//! powered by Patricia trees and inclusion proofs.

use samizdat_common::db::{writable_tx, Droppable, Table as _, WritableTx};
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::fmt::{self, Display};
use std::str::FromStr;

use samizdat_common::{Hash, Hint, PatriciaMap, PatriciaProof, Riddle};

use crate::db::Table;

use super::{ObjectHeader, ObjectRef};

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
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

impl Display for ItemPathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ItemPathBuf {
    /// Transforms into a borrowed item path.
    pub(super) fn as_path(&self) -> ItemPath {
        ItemPath(self.0.as_ref().into())
    }

    /// Hashes this item path.
    fn hash(&self) -> Hash {
        Hash::from_bytes(self.0.as_bytes())
    }
}

/// A borrowed item path.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemPath<'a>(Cow<'a, str>);

impl<'a> From<&'a str> for ItemPath<'a> {
    fn from(name: &'a str) -> ItemPath<'a> {
        ItemPath(normalize(name))
    }
}

impl Display for ItemPath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ItemPath<'_> {
    /// Retrieves the string representation of this path, in its canonical form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An association of paths with all object hashes for a given collection. Inventories
/// are automatically included in all collections as a JSON file under the key
/// `_inventory` in base editions and `_changelogs/<edition timestamp>` in layer editions.
/// They work much the same way like sitemaps do on the regular Web.
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct Inventory {
    /// The mapping of all paths to the corresponding object hash.
    #[serde_as(as = "BTreeMap<_, DisplayFromStr>")]
    inventory: BTreeMap<ItemPathBuf, Hash>,
}

impl FromIterator<(ItemPathBuf, Hash)> for Inventory {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = (ItemPathBuf, Hash)>,
    {
        Inventory {
            inventory: iter.into_iter().collect::<BTreeMap<ItemPathBuf, Hash>>(),
        }
    }
}

impl<'a> IntoIterator for &'a Inventory {
    type IntoIter = std::collections::btree_map::Iter<'a, ItemPathBuf, Hash>;
    type Item = (&'a ItemPathBuf, &'a Hash);
    fn into_iter(self) -> Self::IntoIter {
        self.inventory.iter()
    }
}

impl Inventory {
    /// Creates an object in the database with the provided objects.
    pub fn insert_into_list(
        is_draft: bool,
        name: ItemPathBuf,
        mut objects: Vec<(ItemPathBuf, ObjectRef)>,
    ) -> Result<Vec<(ItemPathBuf, ObjectRef)>, crate::Error> {
        let serialized = serde_json::to_string_pretty(
            &objects
                .iter()
                .map(|(path, object_ref)| (path.clone(), *object_ref.hash()))
                .collect::<Inventory>(),
        )
        .expect("can serialize");

        let object = ObjectRef::build(
            ObjectHeader::new("application/json".to_owned(), is_draft)?,
            false,
            serialized.into_bytes().into_iter().map(Ok),
        )?;

        objects.push((name, object));

        Ok(objects)
    }

    /// Iterates through all key-value pairs in this inventory.
    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }

    /// The number of entries in this inventory.
    pub fn len(&self) -> usize {
        self.inventory.len()
    }

    /// Whether this inventory has any items at all.
    pub fn is_empty(&self) -> bool {
        self.inventory.is_empty()
    }
}

/// An item inside a collection.
#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionItem {
    /// The collection this item belongs to.
    pub collection: CollectionRef,
    /// The path of this item in the collection.
    pub name: ItemPathBuf,
    /// The cryptographic proof that the item is included in the collection.
    pub inclusion_proof: PatriciaProof,
    /// Whether this item is a draft. Drafts cannot be shared with the Samizdat network.
    #[serde(default)]
    pub is_draft: bool,
}

impl Droppable for CollectionItem {
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        let path = self.name.as_path();
        let locator = self.collection.locator_for(path);

        tracing::info!("Removing item {}: {:#?}", locator, self);

        Table::CollectionItemLocators.delete(tx, locator.path());
        Table::CollectionItems.delete(tx, locator.hash());

        Ok(())
    }
}

impl CollectionItem {
    /// Checks whether a collection item is valid, that is
    ///
    /// 1. If the inclusion proof is valid for the claimed collection.
    /// 2. If the claimed name corresponds to the claimed key hash in the inclusion proof.
    pub fn is_valid(&self) -> bool {
        let is_included = self.inclusion_proof.is_in(&self.collection.hash);
        let key = Hash::from_bytes(self.name.0.as_bytes());

        if !is_included {
            tracing::error!("Inclusion proof failed for {:?}", self);
            return false;
        }

        if &key != self.inclusion_proof.claimed_key() {
            tracing::error!("Key is different from claimed key: {:?}", self);
            return false;
        }

        true
    }

    /// Returns an object reference if item is valid. Else, returns
    /// `Err(Error::InvalidCollectionItem)`.
    pub fn object(&self) -> Result<ObjectRef, crate::Error> {
        if self.is_valid() {
            Ok(ObjectRef::new(*self.inclusion_proof.claimed_value()))
        } else {
            Err(crate::Error::InvalidCollectionItem)
        }
    }

    /// Returns the locator of this collection item. The locator works as a URL for items.
    pub fn locator(&self) -> Locator {
        Locator {
            collection: self.collection.clone(),
            name: self.name.as_path(),
        }
    }

    /// Runs through the database trying to find an item that fits to the supplied
    /// content riddle. Returns `Ok(None)` if no matching item is found.
    pub fn find(
        content_riddle: &Riddle,
        hint: &Hint,
    ) -> Result<Option<CollectionItem>, crate::Error> {
        let outcome = Table::CollectionItems
            .prefix(hint.prefix())
            .atomic_for_each(|key, value| {
                let hash: Hash = match key.try_into() {
                    Ok(hash) => hash,
                    Err(err) => {
                        tracing::warn!("{}", err);
                        return None;
                    }
                };

                if content_riddle.resolves(&hash) {
                    let item: CollectionItem =
                        bincode::deserialize(value).expect("can deserialize");

                    match item.object().and_then(|o| o.exists()) {
                        Ok(true) => return Some(Ok(item)),
                        Ok(false) => return None,
                        Err(err) => return Some(Err(err)),
                    }
                }

                None
            })
            .transpose()?;

        Ok(outcome)
    }

    /// Inserts this collection item in the database using the supplied [`Tx`].
    pub fn insert_with(&self, tx: &mut WritableTx<'_>) {
        let locator = self.collection.locator_for(self.name.as_path());

        Table::CollectionItems.put(
            tx,
            locator.hash(),
            bincode::serialize(self).expect("can serialize"),
        );
        Table::CollectionItemLocators.put(tx, locator.path(), locator.hash());

        tracing::info!("Inserting item {}: {:#?}", locator, self);
    }

    /// Inserts this collection item in the database.
    pub fn insert(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| {
            self.insert_with(tx);
            Ok(())
        })
    }
}

/// A reference to a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRef {
    /// The root hash of the Patricia tree of the collection.
    hash: Hash,
}

impl Droppable for CollectionRef {
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        for name in self.list() {
            if let Some(item) = self.get(name.as_path())? {
                item.drop_if_exists_with(tx)?;
            }
        }

        Ok(())
    }
}

impl CollectionRef {
    /// Creates a new collection reference from a root hash of a Patricia tree.
    pub fn new(hash: Hash) -> CollectionRef {
        CollectionRef { hash }
    }

    /// Gets the root hash of this collection.
    pub fn hash(&self) -> Hash {
        self.hash
    }

    /// Generates a collection reference from a random hash.
    #[cfg(test)]
    pub(crate) fn rand() -> CollectionRef {
        CollectionRef { hash: Hash::rand() }
    }

    /// Builds a new collection from a list of paths and objects, returning the collection
    /// reference.
    pub fn build<I>(is_draft: bool, objects: I) -> Result<CollectionRef, crate::Error>
    where
        I: Sync + Send + AsRef<[(ItemPathBuf, ObjectRef)]>,
    {
        // Note: this is the slow part of the process (by a long stretch)
        let patricia_map = objects
            .as_ref()
            .iter()
            .map(|(name, object)| (name.hash(), *object.hash()))
            .collect::<PatriciaMap>();

        let root = *patricia_map.root();
        let collection = CollectionRef { hash: root };

        writable_tx(|tx| {
            for (name, _object) in objects.as_ref() {
                let item = CollectionItem {
                    collection: collection.clone(),
                    name: name.clone(),
                    inclusion_proof: patricia_map
                        .proof_for(name.hash())
                        .expect("name exists in map"),
                    is_draft,
                };

                item.insert_with(tx);
            }

            Ok(collection)
        })
    }

    /// Builds the locator for the supplied item name in the current collection.
    pub fn locator_for<'a>(&self, name: ItemPath<'a>) -> Locator<'a> {
        Locator {
            collection: self.clone(),
            name,
        }
    }

    /// Looks up in the database for an item with the given name in the current
    /// collection. Note that the item must exist in the database for a result to be
    /// returned.
    pub fn get(&self, name: ItemPath) -> Result<Option<CollectionItem>, crate::Error> {
        let locator = self.locator_for(name);
        locator.get()
    }

    /// Returns an iterator over all the item paths for the current collection that
    /// currently exist in the database.
    pub fn list(&'_ self) -> Vec<ItemPathBuf> {
        Table::CollectionItemLocators
            .prefix(self.hash)
            .atomic_collect(|key, _| {
                ItemPathBuf::from(&*String::from_utf8_lossy(&key[self.hash.as_ref().len()..]))
            })
    }

    /// Returns an iterator over all the objects for the current collection that
    /// currently exist in the database.
    pub fn list_objects(&'_ self) -> impl '_ + Iterator<Item = Result<ObjectRef, crate::Error>> {
        self.list().into_iter().filter_map(move |name| {
            let locator = Locator {
                collection: self.clone(),
                name: name.as_path(),
            };
            locator.get_object().transpose()
        })
    }
}

/// A locator works like a URL for collection items.
#[derive(Debug)]
pub struct Locator<'a> {
    /// The collection reference.
    collection: CollectionRef,
    /// The name of the item in the referenced collection.
    name: ItemPath<'a>,
}

impl Display for Locator<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.collection.hash, self.name.0)
    }
}

impl Locator<'_> {
    /// Returns the corresponding hash for this locator.
    pub fn hash(&self) -> Hash {
        self.collection
            .hash
            .rehash(&Hash::from_bytes(self.name.0.as_ref()))
    }

    /// The collection reference for this locator.
    pub fn collection(&self) -> CollectionRef {
        self.collection.clone()
    }

    /// The full key in the database for this locator.
    pub fn path(&self) -> Vec<u8> {
        [self.collection.hash.as_ref(), self.name.0.as_bytes()].concat()
    }

    /// Tries to retrieve the corresponding item from the database.
    pub fn get(&self) -> Result<Option<CollectionItem>, crate::Error> {
        Ok(Table::CollectionItems
            .atomic_get(self.hash(), |item| bincode::deserialize(item))
            .transpose()?)
    }

    /// Tries to retrieve the corresponding object from the database.
    pub fn get_object(&self) -> Result<Option<ObjectRef>, crate::Error> {
        if let Some(item) = self.get()? {
            Ok(Some(item.object()?))
        } else {
            Ok(None)
        }
    }
}
