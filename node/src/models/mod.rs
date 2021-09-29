mod collection;
mod object;
mod series;

pub use collection::{CollectionItem, CollectionRef, Locator};
pub use object::{ObjectMetadata, ObjectRef, ObjectStatistics, ObjectStream, CHUNK_SIZE};
pub use series::{SeriesItem, SeriesOwner, SeriesRef};

// pub enum DynEntity {
//     Object(ObjectRef),
//     Collection(CollectionRef),
//     SeriesOwner(SeriesOwner),
//     Series(SeriesRef),
// }
