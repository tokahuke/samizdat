mod collection;
mod object;
mod series;

pub use collection::{CollectionItem, CollectionRef, Locator};
pub use object::{ObjectMetadata, ObjectRef, ObjectStream, CHUNK_SIZE, ObjectStatistics};
pub use series::{SeriesItem, SeriesOwner, SeriesRef};

// pub enum DynEntity {
//     Object(ObjectRef),
//     Collection(CollectionRef),
//     SeriesOwner(SeriesOwner),
//     Series(SeriesRef),
// }
