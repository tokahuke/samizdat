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
pub use object::{
    ContentStream, ObjectHeader, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior, CHUNK_SIZE,
};
pub use series::{Edition, SeriesOwner, SeriesRef};
pub use subscription::{Subscription, SubscriptionKind, SubscriptionRef};

use rocksdb::WriteBatch;

use crate::db;

/// An object that must be correctly removed from the DB.
pub trait Droppable {
    /// Writes the operations to safely remove the object from the database into the
    /// [`WriteBatch`]. This method should not change the state of the database directly.
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error>;

    /// Safely drops the object from the database.
    fn drop_if_exists(&self) -> Result<(), crate::Error> {
        let mut batch = WriteBatch::default();
        self.drop_if_exists_with(&mut batch)?;
        db().write(batch)?;

        Ok(())
    }
}
