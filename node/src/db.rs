use crate::cli;

static mut DB: Option<rocksdb::DB> = None;

pub fn db<'a>() -> &'a rocksdb::DB {
    unsafe { DB.as_ref().expect("db not initialized") }
}

pub fn init_db() -> Result<(), crate::Error> {
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    let db = rocksdb::DB::open_cf(
        &db_opts,
        &cli().db_path,
        &vec![
            Table::Objects,
            Table::ObjectMetadata,
            Table::ObjectChunks,
            Table::Collections,
            Table::CollectionMetadata,
            Table::CollectionItems,
            Table::CollectionItemMetadata,
            Table::Series,
            Table::SeriesItems,
            Table::SeriesOwners,
            Table::RecentNonces,
        ]
        .into_iter()
        .map(Table::name)
        .collect::<Vec<_>>(),
    )?;

    unsafe {
        DB = Some(db);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum Table {
    /// The list of all inscribed hashes.
    Objects,
    /// The map of all object (out-of-band) metadata, indexed by object hash.
    ObjectMetadata,
    /// The table of all chunks, indexed by chunk hash.
    ObjectChunks,
    /// The list of all known collections.
    Collections,
    /// The list of all collection metadata.
    CollectionMetadata,
    /// The list of all collection items.
    CollectionItems,
    CollectionItemMetadata,
    /// The list of all projects.
    Series,
    SeriesItems,
    SeriesOwners,
    RecentNonces,
}

impl Table {
    pub fn name(self) -> String {
        format!("{:?}", self)
    }

    pub fn get<'a>(self) -> &'a rocksdb::ColumnFamily {
        let db = db();
        db.cf_handle(&format!("{:?}", self))
            .expect("column family exists")
    }
}
