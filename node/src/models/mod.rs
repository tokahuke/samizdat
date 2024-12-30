//! Models for the entities living in the node database.

mod bookmark;
mod collection;
mod hub;
mod object;
mod series;
mod subscription;

pub use bookmark::{Bookmark, BookmarkType};
pub use collection::{CollectionItem, CollectionRef, Inventory, ItemPath, ItemPathBuf, Locator};
pub use hub::Hub;
use jammdb::Tx;
pub use object::{
    get_chunk, ContentStream, ObjectHeader, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior,
    CHUNK_SIZE,
};
pub use series::{Edition, EditionKind, SeriesOwner, SeriesRef};
pub use subscription::{Subscription, SubscriptionKind, SubscriptionRef};

use crate::db::writable_tx;

/// An object that must be correctly removed from the DB.
pub trait Droppable {
    /// Writes the operations to safely remove the object from the database into the
    /// [`WriteBatch`]. This method should not change the state of the database directly.
    fn drop_if_exists_with<'a>(&self, tx: &Tx<'a>) -> Result<(), crate::Error>;

    /// Safely drops the object from the database.
    fn drop_if_exists(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| self.drop_if_exists_with(&tx))
    }
}
