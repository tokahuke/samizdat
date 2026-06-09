//! In-flight chunk protection for the vacuum-vs-import race.
//!
//! `do_import` opens a [`ChunkProtector`] over the chunks it is about
//! to fetch. The orphan-drop path (inline in
//! `ObjectRef::drop_if_exists_with` and in `vacuum::drop_orphan_chunks`)
//! checks [`is_protected`] before deleting a chunk; a protected chunk
//! is skipped, so a concurrent import whose `create_object_with` has
//! not yet bumped the LMDB refcount is not silently corrupted.
//!
//! Crash safety: protection is in-memory only. A process crash
//! evaporates the `PROTECTED` map; nothing to clean up at the cap
//! layer. The startup pass `vacuum::sweep_crash_leaked_chunks` still
//! handles the LMDB side (chunk bytes left behind by imports killed
//! between the per-chunk write and `create_object_with`).
//!
//! ## Lock ordering
//!
//! Two locks interact with this module:
//!
//! - **L_lmdb** -- LMDB's single-writer lock, held for the duration
//!   of any `writable_tx`.
//! - **L_protected** -- the `RwLock` around [`PROTECTED`].
//!
//! The canonical ordering is **L_lmdb -> L_protected**: code that
//! holds `L_lmdb` may acquire `L_protected` (for read or write).
//! Code holding `L_protected` MUST NOT acquire `L_lmdb`. The
//! provided API satisfies this:
//!
//! - [`ChunkProtector::protect`] and [`ChunkProtector::drop`] mutate
//!   only the in-memory `BTreeMap`; no LMDB call while the write
//!   guard is held.
//! - [`is_protected`] takes a `&WritableTx` witness so the compiler
//!   refuses callers who are not already inside an LMDB writer tx.
//!   Without that anchoring, a stale `false` could race a new
//!   `protect` and produce a corrupted object.
//!
//! Violating the ordering opens a deadlock cycle: a holder of
//! `L_protected.write` waiting on `L_lmdb` while a holder of
//! `L_lmdb` waits on `L_protected`. Do not add LMDB calls to
//! `protect`/`drop` without re-evaluating this comment.

use std::collections::BTreeMap;
use std::sync::{LazyLock, RwLock};

use samizdat_common::Hash;
use samizdat_common::db::WritableTx;

/// Process-wide registry of chunks held by in-flight imports.
///
/// `BTreeMap<Hash, usize>` (not `BTreeSet`) so concurrent imports of
/// the same shared chunk refcount independently; an entry only
/// vanishes when the last [`ChunkProtector`] covering that hash
/// drops.
static PROTECTED: LazyLock<RwLock<BTreeMap<Hash, usize>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Owning handle returned by [`ChunkProtector::protect`]. Drop
/// releases the protection for the chunks it covers.
#[must_use = "drop releases the protection; bind to a variable for the import's lifetime"]
pub struct ChunkProtector {
    hashes: Vec<Hash>,
}

impl ChunkProtector {
    /// Mark `hashes` as in-flight. Subsequent calls to
    /// [`is_protected`] for any of them return `true` until this
    /// [`ChunkProtector`] drops.
    pub fn protect(hashes: Vec<Hash>) -> ChunkProtector {
        let mut guard = PROTECTED.write().expect("PROTECTED poisoned");
        for h in &hashes {
            *guard.entry(*h).or_insert(0) += 1;
        }
        ChunkProtector { hashes }
    }
}

impl Drop for ChunkProtector {
    fn drop(&mut self) {
        let mut guard = PROTECTED.write().expect("PROTECTED poisoned");
        for h in &self.hashes {
            if let Some(count) = guard.get_mut(h) {
                *count -= 1;
                if *count == 0 {
                    guard.remove(h);
                }
            }
        }
    }
}

/// Whether `hash` is currently in-flight for some import.
///
/// The unused `&WritableTx` parameter is a compile-time witness that
/// the caller is inside an LMDB writer tx. This matters because the
/// LMDB writer lock is the outer lock that serialises this check
/// against any concurrent `ChunkProtector::protect` whose chunk-write
/// would otherwise race: the import's writable_tx is queued behind
/// the caller's, so the answer this returns is the answer that
/// matters for the caller's delete.
///
/// Without the witness, a caller outside a `writable_tx` could
/// observe a stale `false` immediately after a `protect` landed, and
/// delete a chunk a fresh import was relying on.
pub fn is_protected(_tx: &WritableTx<'_>, hash: &Hash) -> bool {
    PROTECTED
        .read()
        .expect("PROTECTED poisoned")
        .contains_key(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Hash {
        Hash::new([byte; 28])
    }

    #[test]
    fn protect_then_drop_releases() {
        let protector = ChunkProtector::protect(vec![h(1), h(2)]);
        {
            let guard = PROTECTED.read().expect("PROTECTED poisoned");
            assert!(guard.contains_key(&h(1)));
            assert!(guard.contains_key(&h(2)));
        }
        drop(protector);
        let guard = PROTECTED.read().expect("PROTECTED poisoned");
        assert!(!guard.contains_key(&h(1)));
        assert!(!guard.contains_key(&h(2)));
    }

    #[test]
    fn double_protect_same_hash_only_clears_on_second_drop() {
        let a = ChunkProtector::protect(vec![h(42)]);
        let b = ChunkProtector::protect(vec![h(42)]);
        {
            let guard = PROTECTED.read().expect("PROTECTED poisoned");
            assert_eq!(guard.get(&h(42)).copied(), Some(2));
        }
        drop(a);
        {
            let guard = PROTECTED.read().expect("PROTECTED poisoned");
            assert_eq!(guard.get(&h(42)).copied(), Some(1));
        }
        drop(b);
        let guard = PROTECTED.read().expect("PROTECTED poisoned");
        assert!(!guard.contains_key(&h(42)));
    }
}
