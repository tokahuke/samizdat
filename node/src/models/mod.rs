//! Models for the entities living in the node database.

mod bookmark;
mod collection;
mod object;
mod series;

pub use bookmark::{Bookmark, BookmarkType};
pub use collection::{CollectionItem, CollectionRef, ItemPath, ItemPathBuf, Locator};
pub use object::{ObjectHeader, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior, CHUNK_SIZE};
pub use series::{SeriesItem, SeriesOwner, SeriesRef};

use rocksdb::WriteBatch;

use crate::db;

pub trait Dropable {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error>;

    fn drop_if_exists(&self) -> Result<(), crate::Error> {
        let mut batch = WriteBatch::default();
        self.drop_if_exists_with(&mut batch)?;
        db().write(batch)?;

        Ok(())
    }
}
