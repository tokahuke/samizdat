//! The Samizdat Hub database, based on top of RocksDb.

mod migrations;

use jammdb::{Bucket, Tx};
use std::cell::RefCell;
use std::fmt::Display;
use std::ops::RangeBounds;
use std::{collections::BTreeSet, sync::OnceLock};
use strum::IntoEnumIterator;
use strum_macros::{EnumIter, IntoStaticStr};

use crate::CLI;

/// The handle to the RocksDB database.
static DB: OnceLock<jammdb::DB> = OnceLock::new();

/// Retrieves a reference to the RocksDB database. Must be called after initialization.
fn db<'a>() -> &'a jammdb::DB {
    DB.get().expect("database should be initialized first")
}

/// Initializes the RocksDB for use by the Samizdat node.
pub fn init_db() -> Result<(), crate::Error> {
    tracing::info!("Starting jammdb");

    let db_path = format!("{}/main.jammdb", CLI.data);
    let db = jammdb::DB::open(db_path)?;
    let tx = db.tx(true)?;
    let tables = Table::names().collect::<BTreeSet<_>>();

    for table in tables {
        tx.get_or_create_bucket(table)?;
    }

    tx.commit()?;

    DB.set(db).ok();

    // Run possible migrations (needs DB set, but still requires exclusive access):
    tracing::info!("RocksDB up. Running migrations...");
    migrations::migrate()?;
    tracing::info!("... done running all migrations.");

    Ok(())
}

pub fn writable_tx<F, T>(f: F) -> Result<T, crate::Error>
where
    F: FnOnce(&Tx) -> Result<T, crate::Error>,
{
    thread_local! {
        static RUNNING_TX_GUARD: RefCell<bool> = RefCell::new(false);
    }

    /// Guarantees drop even in the presence of a panic.
    struct DeferGuard<'a>(&'a RefCell<bool>);

    impl<'a> Drop for DeferGuard<'a> {
        fn drop(&mut self) {
            // Does not panic if underlying `RefCell` is not borrowed.
            *self.0.borrow_mut() = false;
        }
    }

    impl<'a> DeferGuard<'a> {
        fn new(guard: &'a RefCell<bool>) -> Self {
            if *guard.borrow() {
                panic!("other writable tx already running. This would surely deadlock!");
            }

            *guard.borrow_mut() = true;

            DeferGuard(guard)
        }
    }

    RUNNING_TX_GUARD.with(|guard| {
        let defer_guard = DeferGuard::new(guard);
        let db = db();
        let tx = db.tx(true)?;

        let ret = f(&tx);

        if ret.is_ok() {
            tx.commit()?;
        }

        drop(defer_guard);

        ret
    })
}

pub fn readonly_tx<F, T>(f: F) -> T
where
    F: FnOnce(&Tx) -> T,
{
    let db = db();
    let tx = db.tx(false).expect("cannot create transaction");

    let ret = f(&tx);

    ret
}

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy, EnumIter, IntoStaticStr)]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// The list of applied migrations.
    Migrations,
    /// The list of all recent nonces. This is to mitigate replay attacks.
    RecentNonces,
    /// Blacklisted IP addresses
    BlacklistedIps,
}

impl Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(self))
    }
}

impl Table {
    /// An iterator for all column family names in the database.
    fn names() -> impl Iterator<Item = String> {
        Table::iter().map(|table| table.to_string())
    }

    fn bucket<'a, 'tx>(self, tx: &'a Tx<'tx>) -> Bucket<'a, 'tx> {
        tx.get_bucket(<&'static str>::from(self))
            .expect("bucket should exist")
    }

    /// Gets the underlying column family after database initialization.
    #[must_use]
    pub fn atomic_get<K, F, T>(self, key: K, transform: F) -> Option<T>
    where
        K: AsRef<[u8]>,
        F: FnOnce(&[u8]) -> T,
    {
        readonly_tx(|tx| self.get(tx, key, transform))
    }

    #[must_use]
    pub fn atomic_has<K>(self, key: K) -> bool
    where
        K: AsRef<[u8]>,
    {
        readonly_tx(|tx| self.bucket(tx).get_kv(key).is_some())
    }

    pub fn atomic_put<'a, K, V>(self, key: K, value: V)
    where
        K: jammdb::ToBytes<'a>,
        V: jammdb::ToBytes<'a>,
    {
        let db = db();
        let tx = db.tx(true).expect("cannot create transaction");

        self.put(&tx, key, value);
        tx.commit().expect("should be able to commit");
    }

    pub fn atomic_delete<K>(self, key: K) -> bool
    where
        K: AsRef<[u8]>,
    {
        writable_tx(|tx| Ok(self.delete(tx, key))).unwrap()
    }

    pub fn atomic_map<K, F>(self, key: K, map: F)
    where
        K: AsRef<[u8]> + for<'a> jammdb::ToBytes<'a>,
        F: FnOnce(Option<&[u8]>) -> Vec<u8>,
    {
        writable_tx(|tx| Ok(self.map(tx, key, map))).unwrap()
    }

    #[must_use]
    pub fn get<'tx, K, F, T>(self, tx: &Tx<'tx>, key: K, transform: F) -> Option<T>
    where
        K: AsRef<[u8]>,
        F: FnOnce(&[u8]) -> T,
    {
        let data = self.bucket(tx).get_kv(key)?;
        let value = transform(data.value());

        Some(value)
    }

    pub fn put<'tx, K, V>(self, tx: &Tx<'tx>, key: K, value: V)
    where
        K: jammdb::ToBytes<'tx>,
        V: jammdb::ToBytes<'tx>,
    {
        self.bucket(tx).put(key, value).expect("key was a bucket");
    }

    pub fn delete<'a, K>(self, tx: &Tx<'a>, key: K) -> bool
    where
        K: AsRef<[u8]>,
    {
        let bucket = self.bucket(tx);
        let result = bucket.delete(key);
        let deleted = match result {
            Ok(_) => true,
            Err(jammdb::Error::KeyValueMissing) => false,
            Err(err) => panic!("deleting value got: {err}"),
        };

        deleted
    }

    pub fn map<'a, K, F>(self, tx: &Tx<'a>, key: K, map: F)
    where
        K: AsRef<[u8]> + jammdb::ToBytes<'a>,
        F: FnOnce(Option<&[u8]>) -> Vec<u8>,
    {
        let bucket = self.bucket(tx);
        let new_value = match bucket.get_kv(key.as_ref()) {
            None => map(None),
            Some(kv) => map(Some(kv.value())),
        };
        bucket.put(key, new_value).expect("key was a bucket");
    }

    #[must_use]
    pub fn range<R>(self, range: R) -> TableRange<R>
    where
        R: for<'a> RangeBounds<&'a [u8]>,
    {
        TableRange { table: self, range }
    }

    #[must_use]
    pub fn prefix<P>(self, prefix: P) -> TablePrefix<P>
    where
        P: AsRef<[u8]>,
    {
        TablePrefix {
            table: self,
            prefix,
        }
    }
}

pub struct TableRange<R>
where
    R: for<'a> RangeBounds<&'a [u8]>,
{
    table: Table,
    range: R,
}

impl<R> TableRange<R>
where
    R: for<'a> RangeBounds<&'a [u8]>,
{
    pub fn atomic_for_each<F, T>(self, mut map: F) -> Option<T>
    where
        F: FnMut(&[u8], &[u8]) -> Option<T>,
    {
        readonly_tx(|tx| {
            for kv in self.table.bucket(tx).range(self.range) {
                if let Some(value) = map(kv.kv().key(), kv.kv().value()) {
                    return Some(value);
                }
            }

            None
        })
    }

    #[must_use]
    pub fn atomic_collect<C, F, V>(self, mut map: F) -> C
    where
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        readonly_tx(|tx| {
            self.table
                .bucket(tx)
                .range(self.range)
                .map(|kv| map(kv.kv().key(), kv.kv().value()))
                .collect()
        })
    }

    pub fn for_each<F, T>(self, tx: &Tx<'_>, mut map: F) -> Option<T>
    where
        F: FnMut(&[u8], &[u8]) -> Option<T>,
    {
        for kv in self.table.bucket(tx).range(self.range) {
            if let Some(value) = map(kv.kv().key(), kv.kv().value()) {
                return Some(value);
            }
        }

        None
    }
}

pub struct TablePrefix<P>
where
    P: AsRef<[u8]>,
{
    table: Table,
    prefix: P,
}

impl<P> TablePrefix<P>
where
    P: AsRef<[u8]>,
{
    pub fn atomic_for_each<F, T>(self, mut map: F) -> Option<T>
    where
        F: FnMut(&[u8], &[u8]) -> Option<T>,
    {
        readonly_tx(|tx| {
            for kv in self.table.bucket(tx).range(self.prefix.as_ref()..) {
                if !kv.key().starts_with(self.prefix.as_ref()) {
                    break;
                }

                if let Some(value) = map(kv.kv().key(), kv.kv().value()) {
                    return Some(value);
                }
            }

            None
        })
    }

    #[must_use]
    pub fn atomic_collect<C, F, V>(self, mut map: F) -> C
    where
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        readonly_tx(|tx| {
            self.table
                .bucket(tx)
                .range(self.prefix.as_ref()..)
                .take_while(|kv| kv.key().starts_with(self.prefix.as_ref()))
                .map(|kv| map(kv.kv().key(), kv.kv().value()))
                .collect()
        })
    }

    pub fn atomic_delete(self) {
        writable_tx(|tx| {
            let bucket = self.table.bucket(tx);
            // cannot delete while iterating! see https://github.com/pjtatlow/jammdb/issues/34
            let mut to_delete = vec![];

            for item in bucket.range(self.prefix.as_ref()..) {
                if !item.key().starts_with(self.prefix.as_ref()) {
                    break;
                }

                to_delete.push(item.key().to_vec());
            }

            for key in to_delete {
                bucket.delete(key).expect("can delete");
            }

            Ok(())
        })
        .expect("infallible");
    }

    pub fn for_each<F, T>(self, tx: &Tx<'_>, mut map: F) -> Option<T>
    where
        F: FnMut(&[u8], &[u8]) -> Option<T>,
    {
        for kv in self.table.bucket(tx).range(self.prefix.as_ref()..) {
            if !kv.key().starts_with(self.prefix.as_ref()) {
                break;
            }

            if let Some(value) = map(kv.kv().key(), kv.kv().value()) {
                return Some(value);
            }
        }

        None
    }
}
