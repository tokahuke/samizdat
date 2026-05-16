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
    /// The LMDB environment handle
    environment: lmdb::Environment,
    /// Collection of database table handles
    tables: Vec<lmdb::Database>,
    /// Type identifier for the table type used to initialize the database. This is used
    /// only to check for consistent use of table t
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

/// A writable transaction handle for the database.
pub struct WritableTx<'tx>(lmdb::RwTransaction<'tx>);

/// A read-only transaction handle for the database.
pub struct ReadonlyTx<'tx>(lmdb::RoTransaction<'tx>);

/// Defines common functionality for transaction handles.
pub trait TxHandle {
    /// The underlying LMDB transaction type.
    type TxType: Transaction;
    /// Returns a reference to the underlying transaction.
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

/// Executes a function within a writable transaction context.
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
                .map_err(|err| format!("cannot create writable transaction: {err}"))?,
        );
        let start = Instant::now();

        let ret = f(&mut tx);

        if start.elapsed() > Duration::from_millis(100) {
            tracing::warn!("long running writable tx took {:?}", start.elapsed())
        }

        let outcome = match ret {
            Ok(value) => tx
                .0
                .commit()
                .map(|()| value)
                .map_err(|err| format!("cannot commit writable transaction: {err}").into()),
            Err(err) => Err(err),
        };

        drop(defer_guard);

        outcome
    })
}

/// Executes a function within a read-only transaction context.
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
            tracing::warn!("long running readonly tx took {:?}", start.elapsed())
        }

        if let Err(err) = tx.0.commit() {
            // RO commit just releases the reader slot; this should not fail in practice.
            // We log loudly and continue rather than aborting; the read already happened.
            tracing::error!("readonly tx commit failed: {err}");
        }

        drop(defer_guard);

        ret
    })
}

/// Defines the interface for database tables.
///
/// All accessor methods return `Result`; LMDB errors (MAP_FULL, OS I/O, etc.) propagate
/// to the caller instead of panicking the process.
pub trait Table: Copy + strum::VariantArray + Into<&'static str> {
    /// The table containing migration records.
    const MIGRATIONS: Self;
    /// Returns the base migration for this table type.
    fn base_migration() -> Box<dyn Migration<Self>>;
    /// Returns the numeric discriminant for this table variant.
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

    /// Looks up `key`, runs `transform` over the bytes (if present), and propagates both
    /// LMDB and closure errors. Returns `Ok(None)` if the key is absent.
    #[inline]
    fn get<Tx, K, F, T>(self, tx: &Tx, key: K, transform: F) -> Result<Option<T>, crate::Error>
    where
        Tx: TxHandle,
        K: AsRef<[u8]>,
        F: FnOnce(&[u8]) -> Result<T, crate::Error>,
    {
        match tx.get_tx().get(self.get_handle(), &key) {
            Ok(data) => Ok(Some(transform(data)?)),
            Err(lmdb::Error::NotFound) => Ok(None),
            Err(err) => Err(format!("db get failed: {err}").into()),
        }
    }

    #[inline]
    fn has<Tx, K>(self, tx: &Tx, key: K) -> Result<bool, crate::Error>
    where
        Tx: TxHandle,
        K: AsRef<[u8]>,
    {
        match tx.get_tx().get(self.get_handle(), &key) {
            Ok(_) => Ok(true),
            Err(lmdb::Error::NotFound) => Ok(false),
            Err(err) => Err(format!("db has failed: {err}").into()),
        }
    }

    #[inline]
    fn put<K, V>(self, tx: &mut WritableTx, key: K, value: V) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        tx.0.put(self.get_handle(), &key, &value, WriteFlags::default())
            .map_err(|err| format!("db put failed: {err}").into())
    }

    /// Deletes `key`. Returns `Ok(true)` if the key existed, `Ok(false)` otherwise.
    #[inline]
    fn delete<K>(self, tx: &mut WritableTx, key: K) -> Result<bool, crate::Error>
    where
        K: AsRef<[u8]>,
    {
        match tx.0.del(self.get_handle(), &key, None) {
            Ok(()) => Ok(true),
            Err(lmdb::Error::NotFound) => Ok(false),
            Err(err) => Err(format!("db delete failed: {err}").into()),
        }
    }

    /// Reads `key`, runs `map_fn` against `Some(bytes)` or `None`, and writes back the
    /// resulting `Vec<u8>` under the same key. The classic read-modify-write helper.
    #[inline]
    fn map<K, F>(self, tx: &mut WritableTx, key: K, map_fn: F) -> Result<(), crate::Error>
    where
        K: AsRef<[u8]>,
        F: FnOnce(Option<&[u8]>) -> Vec<u8>,
    {
        let new_value = match tx.get_tx().get(self.get_handle(), &key) {
            Ok(data) => map_fn(Some(data)),
            Err(lmdb::Error::NotFound) => map_fn(None),
            Err(err) => return Err(format!("db map (read step) failed: {err}").into()),
        };
        tx.0.put(self.get_handle(), &key, &new_value, WriteFlags::default())
            .map_err(|err| format!("db map (write step) failed: {err}").into())
    }

    #[inline]
    #[must_use]
    fn range<R, K>(self, range: R) -> TableRange<Self, R, K>
    where
        K: AsRef<[u8]>,
        R: RangeBounds<K>,
    {
        TableRange {
            table: self,
            range,
            _key: std::marker::PhantomData,
        }
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

/// A range-based iterator over table entries.
pub struct TableRange<T, R, K>
where
    T: Table,
    K: AsRef<[u8]>,
    R: RangeBounds<K>,
{
    /// The table to iterate over
    table: T,
    /// The range bounds for iteration
    range: R,
    _key: std::marker::PhantomData<K>,
}

impl<T, R, K> TableRange<T, R, K>
where
    T: Table,
    K: AsRef<[u8]>,
    R: RangeBounds<K>,
{
    /// Iterates over entries in range; the closure returns `Ok(Some(value))` to break
    /// early, `Ok(None)` to continue, or `Err(_)` to abort with that error. Returns the
    /// break value if any, or `None` after exhausting the range.
    pub fn for_each<Tx, F, U>(
        self,
        tx: &Tx,
        mut map: F,
    ) -> Result<Option<U>, crate::Error>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> Result<Option<U>, crate::Error>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .map_err(|err| format!("open_ro_cursor failed: {err}"))?;
        let iter = match self.range.start_bound() {
            Bound::Included(start) => cursor.iter_from(start),
            Bound::Excluded(_) => return Err("excluded start bound not supported".into()),
            Bound::Unbounded => cursor.iter_start(),
        };

        for item in iter {
            let (key, value) = item.map_err(|err| format!("cursor advance failed: {err}"))?;

            let past_end = match self.range.end_bound() {
                Bound::Included(end) => key > end.as_ref(),
                Bound::Excluded(end) => key >= end.as_ref(),
                Bound::Unbounded => false,
            };
            if past_end {
                break;
            }

            if let Some(value) = map(key, value)? {
                return Ok(Some(value));
            }
        }

        Ok(None)
    }

    pub fn collect<Tx, C, F, V>(self, tx: &Tx, mut map: F) -> Result<C, crate::Error>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .map_err(|err| format!("open_ro_cursor failed: {err}"))?;
        let iter = match self.range.start_bound() {
            Bound::Included(start) => cursor.iter_from(start),
            Bound::Excluded(_) => return Err("excluded start bound not supported".into()),
            Bound::Unbounded => cursor.iter_start(),
        };

        let mut items: Vec<V> = Vec::new();
        for item in iter {
            let (key, value) = item.map_err(|err| format!("cursor advance failed: {err}"))?;
            let past_end = match self.range.end_bound() {
                Bound::Included(end) => key > end.as_ref(),
                Bound::Excluded(end) => key >= end.as_ref(),
                Bound::Unbounded => false,
            };
            if past_end {
                break;
            }
            items.push(map(key, value));
        }
        Ok(C::from_iter(items))
    }
}

/// A prefix-based iterator over table entries.
pub struct TablePrefix<T, P>
where
    T: Table,
    P: AsRef<[u8]>,
{
    /// The table to iterate over
    table: T,
    /// The prefix to filter entries by
    prefix: P,
}

impl<T, P> TablePrefix<T, P>
where
    T: Table,
    P: AsRef<[u8]>,
{
    pub fn delete(self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        let mut to_delete = vec![];
        {
            let mut cursor =
                tx.0.open_rw_cursor(self.table.get_handle())
                    .map_err(|err| format!("open_rw_cursor failed: {err}"))?;
            for item in cursor.iter_from(self.prefix.as_ref()) {
                let (key, _) = item.map_err(|err| format!("cursor advance failed: {err}"))?;
                if !key.starts_with(self.prefix.as_ref()) {
                    break;
                }
                to_delete.push(key.to_vec());
            }
        }

        for key in to_delete {
            self.table.delete(tx, key)?;
        }
        Ok(())
    }

    pub fn for_each<Tx, F, U>(
        self,
        tx: &Tx,
        mut map: F,
    ) -> Result<Option<U>, crate::Error>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> Result<Option<U>, crate::Error>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .map_err(|err| format!("open_ro_cursor failed: {err}"))?;

        for item in cursor.iter_from(self.prefix.as_ref()) {
            let (key, value) = item.map_err(|err| format!("cursor advance failed: {err}"))?;
            if !key.starts_with(self.prefix.as_ref()) {
                break;
            }
            if let Some(value) = map(key, value)? {
                return Ok(Some(value));
            }
        }

        Ok(None)
    }

    pub fn collect<Tx, C, F, V>(self, tx: &Tx, mut map: F) -> Result<C, crate::Error>
    where
        Tx: TxHandle,
        F: FnMut(&[u8], &[u8]) -> V,
        C: FromIterator<V>,
    {
        let mut cursor = tx
            .get_tx()
            .open_ro_cursor(self.table.get_handle())
            .map_err(|err| format!("open_ro_cursor failed: {err}"))?;

        let mut items: Vec<V> = Vec::new();
        for item in cursor.iter_from(self.prefix.as_ref()) {
            let (key, value) = item.map_err(|err| format!("cursor advance failed: {err}"))?;
            if !key.starts_with(self.prefix.as_ref()) {
                break;
            }
            items.push(map(key, value));
        }
        Ok(C::from_iter(items))
    }
}

/// A migration to be run in the database at process start.
pub trait Migration<T>: Debug
where
    T: Table,
{
    fn next(&self) -> Option<Box<dyn Migration<T>>>;
    fn up(&self, tx: &mut WritableTx) -> Result<(), crate::Error>;

    fn is_up(&self, tx: &mut WritableTx) -> Result<bool, crate::Error> {
        let migration_key = format!("{self:?}");
        T::MIGRATIONS.has(tx, migration_key)
    }

    fn migrate(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| {
            if !self.is_up(tx)? {
                let migration_key = format!("{self:?}");

                // Both the migration and the bookkeeping write happen in the same
                // writable_tx, so they commit atomically (or roll back together).
                tracing::info!("Applying migration {self:?}...");
                self.up(tx)?;
                T::MIGRATIONS.put(tx, migration_key, [])?;
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

/// Test harness for the global LMDB singleton.
///
/// `init_db` can only be called once per process (the `OnceLock` is set forever), which
/// makes the production API hostile to unit testing. This module provides:
///
///   * [`TestDb`]; a fresh per-test LMDB environment in a `tempfile::TempDir`. Multiple
///     `TestDb` instances inside a single test binary share the global slot; the FIRST
///     `TestDb::new::<T>()` initializes it; subsequent calls return a new tempdir but
///     reuse that handle (callers MUST clear keys between tests for isolation, or use a
///     single test that exercises everything).
///   * [`TestDb::with`]; convenience for grabbing the harness, running a closure that
///     uses `writable_tx` / `readonly_tx`, and returning the result.
///
/// Because LMDB writes are global once initialized, tests that need a truly clean slate
/// should run in separate binaries (e.g. `tests/foo.rs`) or use `cargo test --test`. For
/// unit tests inside `common`/`node`/`hub`, treat the in-process DB as additive and
/// either: (a) keep tests independent of cross-test side effects, or (b) drop the data
/// you wrote at the end of each test.
#[cfg(any(test, feature = "test-helpers"))]
pub mod test_harness {
    use super::*;
    use std::sync::Mutex;

    /// Process-wide guard so concurrent tests serialise their DB access.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// A handle to a per-process test database, backed by a `tempfile::TempDir`.
    pub struct TestDb<T: Table> {
        _tempdir: tempfile::TempDir,
        _phantom: std::marker::PhantomData<T>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl<T: Table + 'static> TestDb<T> {
        /// Initializes the global DB on a fresh tempdir if it isn't initialized yet, or
        /// asserts that the existing init matches `T`. Returns a handle that holds the
        /// process-wide test lock for serialised access.
        ///
        /// # Panics
        ///
        /// Panics if a previous call initialized the DB with a different `Table` type.
        pub fn new() -> Self {
            let guard = TEST_LOCK
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let tempdir = tempfile::TempDir::new().expect("create tempdir");

            if DB.get().is_none() {
                let database = Database::init::<T>(&tempdir.path().to_string_lossy())
                    .expect("init test DB");
                DB.set(database).ok();
                T::base_migration()
                    .migrate()
                    .expect("run test DB migrations");
            } else {
                assert_eq!(
                    DB.get().expect("just checked").table_type,
                    TypeId::of::<T>(),
                    "TestDb<T> requested a different Table type than the one already \
                     initialized in this process. Run conflicting tests in separate \
                     binaries (tests/*.rs)."
                );
            }

            TestDb {
                _tempdir: tempdir,
                _phantom: std::marker::PhantomData,
                _guard: guard,
            }
        }

        /// Runs `f` against the test DB and returns its result. Holds the test lock for
        /// the duration so concurrent tests don't interleave writes.
        pub fn with<R>(f: impl FnOnce() -> R) -> R {
            let _db = Self::new();
            f()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum TestTable {
        Migrations,
        ScratchA,
        ScratchB,
    }

    impl strum::VariantArray for TestTable {
        const VARIANTS: &'static [Self] = &[
            TestTable::Migrations,
            TestTable::ScratchA,
            TestTable::ScratchB,
        ];
    }

    impl From<TestTable> for &'static str {
        fn from(t: TestTable) -> &'static str {
            match t {
                TestTable::Migrations => "migrations",
                TestTable::ScratchA => "scratch_a",
                TestTable::ScratchB => "scratch_b",
            }
        }
    }

    #[derive(Debug)]
    struct BaseMigration;
    impl Migration<TestTable> for BaseMigration {
        fn next(&self) -> Option<Box<dyn Migration<TestTable>>> {
            None
        }
        fn up(&self, _tx: &mut WritableTx) -> Result<(), crate::Error> {
            Ok(())
        }
    }

    impl Table for TestTable {
        const MIGRATIONS: Self = TestTable::Migrations;
        fn base_migration() -> Box<dyn Migration<Self>> {
            Box::new(BaseMigration)
        }
        fn discriminant(self) -> usize {
            self as usize
        }
    }

    use test_harness::TestDb;

    /// Sanity check: the harness can init, put, get, delete; round-trips correctly.
    #[test]
    fn harness_round_trip() {
        TestDb::<TestTable>::with(|| {
            writable_tx(|tx| {
                TestTable::ScratchA.put(tx, b"k", b"v")?;
                Ok(())
            })
            .unwrap();

            let got = readonly_tx(|tx| {
                TestTable::ScratchA.get(tx, b"k", |bytes| Ok(bytes.to_vec()))
            })
            .unwrap();
            assert_eq!(got, Some(b"v".to_vec()));

            writable_tx(|tx| {
                let deleted = TestTable::ScratchA.delete(tx, b"k")?;
                assert!(deleted);
                Ok(())
            })
            .unwrap();

            let after = readonly_tx(|tx| {
                TestTable::ScratchA.has(tx, b"k")
            })
            .unwrap();
            assert!(!after);
        });
    }

    /// Regression test for P9; a closure returning an error from a writable_tx must
    /// propagate that error, NOT panic.
    #[test]
    fn writable_tx_propagates_closure_error() {
        TestDb::<TestTable>::with(|| {
            let result: Result<(), crate::Error> = writable_tx(|_tx| Err("nope".into()));
            assert!(result.is_err());
        });
    }

    /// Regression test for P9; `get` with a fallible transform returns the closure's
    /// error rather than panicking.
    #[test]
    fn get_propagates_transform_error() {
        TestDb::<TestTable>::with(|| {
            writable_tx(|tx| {
                TestTable::ScratchB.put(tx, b"bad", &[0u8, 1, 2])?;
                Ok(())
            })
            .unwrap();

            let outcome: Result<Option<u64>, crate::Error> = readonly_tx(|tx| {
                TestTable::ScratchB.get(tx, b"bad", |bytes| {
                    // Deliberately fail.
                    Err(format!("can't decode {} bytes", bytes.len()).into())
                })
            });
            assert!(outcome.is_err(), "closure error did not propagate");
        });
    }

    /// Regression test for P9; `range::for_each` surfaces closure errors.
    #[test]
    fn range_for_each_propagates_error() {
        TestDb::<TestTable>::with(|| {
            writable_tx(|tx| {
                for i in 0u8..5 {
                    TestTable::ScratchA.put(tx, [i], [i])?;
                }
                Ok(())
            })
            .unwrap();

            let outcome: Result<Option<()>, crate::Error> = readonly_tx(|tx| {
                TestTable::ScratchA
                    .range::<_, [u8; 1]>(..)
                    .for_each(tx, |_, _| Err("boom".into()))
            });
            assert!(outcome.is_err());
        });
    }
}
