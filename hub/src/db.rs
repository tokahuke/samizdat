use crate::CLI;

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
        &format!("{}/db", CLI.data),
        &vec![Table::RecentNonces]
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
    /// The list of all recent nonces. This is to mitigate replay attacks.
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
