//! The Samizdat Hub database, based on top of RocksDb.

use lmdb::{Cursor, DatabaseFlags, Transaction, WriteFlags};
use std::any::TypeId;
use std::cell::RefCell;
use std::fmt::Debug;
use std::ops::{Bound, RangeBounds};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Maximum lmdb size. Approx. 68GB; may be changed later...
const MAP_SIZE: usize = 1 << 36;
static DB: OnceLock<Database> = OnceLock::new();

/// Initializes the database at the suppiled path using the supplied table type. This
/// function can only be called once per process and will panic at the second and
/// consecutive invocations.
pub fn init_db<T: Table>(root: &str) -> Result<(), crate::Error> {
    let database = Database::init::<T>(root)?;
    DB.set(database).expect("DB was already initialized");

    // Run possible migrations (needs DB set, but still requires exclusive access):
    tracing::info!("running migrations...");
    T::base_migration().migrate()?;
    tracing::info!("... done running all migrations.");

    Ok(())
}

fn db<'a>() -> &'a Database {
    DB.get().expect("DB was not initialized")
}

#[derive(Debug)]
struct Database {
    environment: lmdb::Environment,
    tables: Vec<lmdb::Database>,
    table_type: TypeId,
}

impl Database {
    fn init<T: Table>(root: &str) -> Result<Database, crate::Error> {
        let db_path = format!("{root}/lmdb");
        tracing::info!("starting LMDB on {db_path}");
        std::fs::create_dir_all(&db_path)?;
        let environment = lmdb::Environment::new()
            .set_max_dbs(T::VARIANTS.len() as u32)
            .set_map_size(MAP_SIZE)
            .open(db_path.as_ref())
            .map_err(|err| format!("failed to start LMDB: {err}"))?;

        let mut tables = vec![];

        for table in T::VARIANTS {
            let name = (*table).into();
            let handle = environment
                .create_db(Some(name), DatabaseFlags::default())
                .map_err(|err| format!("failed to open table {name}: {err}"))?;
            tables.push(handle);
        }

        Ok(Database {
            environment,
            tables,
            table_type: TypeId::of::<T>(),
        })
    }
}

pub struct WritableTx<'tx>(lmdb::RwTransaction<'tx>);

pub struct ReadonlyTx<'tx>(lmdb::RoTransaction<'tx>);

pub trait TxHandle {
    type TxType: Transaction;
    fn get_tx(&self) -> &Self::TxType;
}

impl<'tx> TxHandle for WritableTx<'tx> {
    type TxType = lmdb::RwTransaction<'tx>;
    fn get_tx(&self) -> &Self::TxType {
        &self.0
    }
}

impl<'tx> TxHandle for ReadonlyTx<'tx> {
    type TxType = lmdb::RoTransaction<'tx>;
    fn get_tx(&self) -> &Self::TxType {
        &self.0
    }
}

#[inline]
pub fn writable_tx<F, T>(f: F) -> Result<T, crate::Error>
where
    F: FnOnce(&mut WritableTx) -> Result<T, crate::Error>,
{
    thread_local! {
        static RUNNING_TX_GUARD: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Guarantees drop even in the presence of a panic.
    struct DeferGuard<'a>(&'a RefCell<bool>);

    impl Drop for DeferGuard<'_> {
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
        let mut tx = WritableTx(
            db().environment
                .begin_rw_txn()
                .expect("cannot create writable transaction"),
        );
        let start = Instant::now();

        let ret = f(&mut tx);

        if start.elapsed() > Duration::from_millis(100) {
            tracing::warn!("long running writable tx took {:?}", start.elapsed())
        }

        if ret.is_ok() {
            tx.0.commit().expect("cannot commit writable transaction");
        }

        drop(defer_guard);

        ret
    })
}

#[inline]
pub fn readonly_tx<F, T>(f: F) -> T
where
    F: FnOnce(&ReadonlyTx) -> T,
{
    thread_local! {
        static RUNNING_TX_GUARD: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Guarantees drop even in the presence of a panic.
    struct DeferGuard<'a>(&'a RefCell<bool>);

    impl Drop for DeferGuard<'_> {
        fn drop(&mut self) {
            // Does not panic if underlying `RefCell` is not borrowed.
            *self.0.borrow_mut() = false;
        }
    }

    impl<'a> DeferGuard<'a> {
        fn new(guard: &'a RefCell<bool>) -> Self {
            if *guard.borrow() {
                panic!("other readonly tx already running. This might lead to errors!");
            }

            *guard.borrow_mut() = true;

            DeferGuard(guard)
        }
    }

    RUNNING_TX_GUARD.with(|guard| {
        let defer_guard = DeferGuard::new(guard);
        let tx = ReadonlyTx(
            db().environment
                .begin_ro_txn()
                .expect("cannot create readonly transaction"),
        );
        let start = Instant::now();

        let ret = f(&tx);

        if start.elapsed() > Duration::from_millis(100) {
            tracing::warn!("long running writable tx took {:?}", start.elapsed())
        }

        tx.0.commit().expect("cannot commit readonly transaction");

        drop(defer_guard);

        ret
    })
}

pub trait Table: Copy + strum::VariantArray + Into<&'static str> {
    const MIGRATIONS: Self;
    fn base_migration() -> Box<dyn Migration<Self>>;
    fn discriminant(self) -> usize;

    #[inline]
    fn get_handle(self) -> lmdb::Database {
        assert_eq!(
            TypeId::of::<Self>(),
            db().table_type,
            "getting handle of a type different from what the database was initialized on",
        );
        db().tables[self.discriminant()]
    }

    #[inline]
    #[must_use]
    fn get<Tx, K, F, T>(self, tx: &Tx, key: K, transform: F) -> Option<T>
    where
        Tx: TxHandle,
        K: AsRef<[u8]>,
        F: FnOnce(&[u8]) -> T,
    {
        let result = tx.get_tx().get(self.get_handle(), &key);

        match result {
            Ok(data) => Some(transform(data)),
            Err(lmdb::Error::NotFound) => None,
            Err(err) => panic!("getting value got: {err}"),
        }
    }

    #[inline]
    #[must_use]
    fn has<Tx, K>(self, tx: &Tx, key: K) -> bool
    where
        Tx: TxHandle,
        K: AsRef<[u8]>,
    {
        self.get(tx, key, |_| ()).is_some()
    }

    #[inline]
    fn put<K, V>(self, tx: &mut WritableTx, key: K, value: V)
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        tx.0.put(self.get_handle(), &key, &value, WriteFlags::default())
            .expect("unable to perform put");
    }

    #[inline]
    fn delete<K>(self, tx: &mut WritableTx, key: K) -> bool
    where
        K: AsRef<[u8]>,
    {
        let result = tx.0.del(self.get_handle(), &key, None);
        match result {
            Ok(_) => true,
            Err(lmdb::Error::NotFound) => false,
            Err(err) => panic!("deleting value got: {err}"),
        }
    }

    #[inline]
    fn map<K, F>(self, tx: &mut WritableTx, key: K, map: F)
    where
        K: AsRef<[u8]>,
        F: FnOnce(Option<&[u8]>) -> Vec<u8>,
    {
        let result = tx.get_tx().get(self.get_handle(), &key);
        let new_value = match result {
            Ok(data) => map(Some(data)),
            Err(lmdb::Error::NotFound) => map(None),
            Err(err) => panic!("mapping value got: {err}"),
        };
        tx.0.put(self.get_handle(), &key, &new_value, WriteFlags::default())
            .expect("unable to perform put");
    }

    #[inline]
    #[must_use]
    fn range<R>(self, range: R) -> TableRange<Self, R>
    where
        R: for<'a> RangeBounds<&'a [u8]>,
    {
        TableRange { table: self, range }
    }

    #[inline]
    #[must_use]
    fn prefix<P>(self, prefix: P) -> TablePrefix<Self, P>
    where
        P: AsRef<[u8]>,
    {
        TablePrefix {
            table: self,
            prefix,
        }
    }
}

pub struct TableRange<T, R>
where
    T: Table,
    R: for<'a> RangeBounds<&'a [u8]>,
{
    table: T,
    range: R,
}

impl<T, R> TableRange<T, R>
where
    T: Table,
    R: for<'a> RangeBounds<&'a [u8]>,
{
    pub fn for_each<Tx, F, U>(self, tx: &Tx, mut map: F) -> Option<U>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> Option<U>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .expect("failed to open readonly cursor");
        let iter = match self.range.start_bound() {
            Bound::Included(start) => cursor.iter_from(start),
            Bound::Excluded(_) => panic!("not supported"),
            Bound::Unbounded => cursor.iter_start(),
        };

        match self.range.end_bound() {
            Bound::Included(end) => {
                for item in iter {
                    let (key, value) = item.expect("unable to advance cursor");

                    if key > *end {
                        break;
                    }

                    if let Some(value) = map(key, value) {
                        return Some(value);
                    }
                }
            }
            Bound::Excluded(end) => {
                for item in iter {
                    let (key, value) = item.expect("unable to advance cursor");

                    if key >= *end {
                        break;
                    }

                    if let Some(value) = map(key, value) {
                        return Some(value);
                    }
                }
            }
            Bound::Unbounded => {
                for item in iter {
                    let (key, value) = item.expect("unable to advance cursor");
                    if let Some(value) = map(key, value) {
                        return Some(value);
                    }
                }
            }
        }

        None
    }

    #[must_use]
    pub fn collect<Tx, C, F, V>(self, tx: &Tx, mut map: F) -> C
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .expect("failed to open readonly cursor");
        let iter = match self.range.start_bound() {
            Bound::Included(start) => cursor.iter_from(start),
            Bound::Excluded(_) => panic!("not supported"),
            Bound::Unbounded => cursor.iter_start(),
        };

        match self.range.end_bound() {
            Bound::Included(end) => C::from_iter(
                iter.map(|item| item.expect("unable to advance cursor"))
                    .take_while(|(key, _)| key <= end)
                    .map(|(key, value)| map(key, value)),
            ),
            Bound::Excluded(end) => C::from_iter(
                iter.map(|item| item.expect("unable to advance cursor"))
                    .take_while(|(key, _)| key < end)
                    .map(|(key, value)| map(key, value)),
            ),
            Bound::Unbounded => C::from_iter(
                iter.map(|item| item.expect("unable to advance cursor"))
                    .map(|(key, value)| map(key, value)),
            ),
        }
    }
}

pub struct TablePrefix<T, P>
where
    T: Table,
    P: AsRef<[u8]>,
{
    table: T,
    prefix: P,
}

impl<T, P> TablePrefix<T, P>
where
    T: Table,
    P: AsRef<[u8]>,
{
    pub fn delete(self, tx: &mut WritableTx) {
        let mut cursor =
            tx.0.open_rw_cursor(self.table.get_handle())
                .expect("failed to open writable cursor");
        // cannot delete while iterating!
        let mut to_delete = vec![];

        for item in cursor.iter_from(self.prefix.as_ref()) {
            let (key, _) = item.expect("unable to advance cursor");

            if !key.starts_with(self.prefix.as_ref()) {
                break;
            }

            to_delete.push(key);
        }

        drop(cursor);

        for key in to_delete {
            self.table.delete(tx, key);
        }
    }

    pub fn for_each<Tx, F, U>(self, tx: &Tx, mut map: F) -> Option<U>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> Option<U>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .expect("failed to open readonly cursor");

        for item in cursor.iter_from(self.prefix.as_ref()) {
            let (key, value) = item.expect("unable to advance cursor");

            if !key.starts_with(self.prefix.as_ref()) {
                break;
            }

            if let Some(value) = map(key, value) {
                return Some(value);
            }
        }

        None
    }

    #[must_use]
    pub fn collect<Tx, C, F, V>(self, tx: &Tx, mut map: F) -> C
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .expect("failed to open readonly cursor");

        C::from_iter(
            cursor
                .iter_from(self.prefix.as_ref())
                .map(|item| item.expect("unable to advance cursor"))
                .take_while(|(key, _)| key.starts_with(self.prefix.as_ref()))
                .map(|(key, value)| map(key, value)),
        )
    }
}

/// A migration to be run in the database at process start.
pub trait Migration<T>: Debug
where
    T: Table,
{
    fn next(&self) -> Option<Box<dyn Migration<T>>>;
    fn up(&self, tx: &mut WritableTx) -> Result<(), crate::Error>;

    fn is_up(&self, tx: &mut WritableTx) -> bool {
        let migration_key = format!("{self:?}");
        T::MIGRATIONS.has(tx, migration_key)
    }

    fn migrate(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| {
            if !self.is_up(tx) {
                let migration_key = format!("{self:?}");

                // This should be atomic, but... oh! dear...
                tracing::info!("Applying migration {self:?}...");
                self.up(tx)?;
                T::MIGRATIONS.put(tx, migration_key, []);
                tracing::info!("... done.");
            } else {
                tracing::info!("Migration {self:?} already up.");
            }

            Ok(())
        })?;

        // Tail-recurse:
        if let Some(last) = self.next() {
            last.migrate()?;
        }

        Ok(())
    }
}

/// An object that must be correctly removed from the DB.
pub trait Droppable {
    /// Writes the operations to safely remove the object from the database into the
    /// [`WriteBatch`]. This method should not change the state of the database directly.
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error>;

    /// Safely drops the object from the database.
    fn drop_if_exists(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| self.drop_if_exists_with(tx))
    }
}
