//! Tools for replay attacks resistance. This avoids both malicious attacks and
//! unfortunate accidents and is also useful to avoid message amplification attacks
//! to the network.
//!
//! # Threat model and limits
//!
//! `check` records every observed nonce for `TOLERATED_AGE`. Within that window,
//! exact replays of a captured message are rejected. After eviction the same bytes
//! could be replayed again; the messages do not carry a signed timestamp, so a
//! truly fresh-looking replay cannot be cryptographically distinguished from a
//! genuine retransmit. The actual safeguards against repeated abuse are the
//! per-node throttle and call-semaphore (see `hub_server::HubServer`); replay
//! resistance here exists to prevent same-burst amplification and accidental
//! duplicates, not long-term unforgeability.

use std::convert::TryInto;
use tokio::time::{interval, Duration};

use samizdat_common::db::{writable_tx, Table as _};
use samizdat_common::rpc::{
    EditionAnnouncement, EditionRequest, IdentityRequest, Query, Resolution,
};
use samizdat_common::Hash;

use crate::db::Table;

/// Maximum age for a nonce. 10min allows for some sloppy clocks out there.
const TOLERATED_AGE: i64 = 600;

/// A type that has a nonce (a "number used only once") associated to it.
pub trait Nonce {
    /// Retrieves the nonce associated with this type.
    fn nonce(&self) -> Hash;
}

/// A service for replay attack resistance.
///
/// Stateless aside from the LMDB-backed nonce set. Methods take `&self` and may be
/// invoked concurrently; the atomicity of `check` comes from doing the read-then-
/// write inside a single `writable_tx` (LMDB serialises writers). The previous
/// design wrapped `ReplayResistance` in a `tokio::sync::Mutex` and serialised
/// every RPC through that mutex while it also performed two separate DB
/// transactions per call; under load that turned the hub into a one-RPC-at-a-time
/// bottleneck and one slow-disk event into a global stall.
pub struct ReplayResistance;

impl ReplayResistance {
    /// Creates a new ReplayResistance service.
    pub fn new() -> ReplayResistance {
        // Cleanup old nonces. Made deliberately infrequent.
        tokio::spawn(async move {
            // Twice would suffice, but thrice is certainty.
            let mut interval = interval(Duration::from_secs(TOLERATED_AGE as u64 * 3));

            loop {
                let _ = run_cleanup_pass();
                interval.tick().await;
            }
        });

        ReplayResistance
    }

    /// Checks whether a nonce has been recently seen and, if not, records it.
    /// Returns `true` if the message is fresh (caller should proceed) and `false`
    /// if it is a replay (caller should reject).
    ///
    /// The check and insert happen in a single writable transaction so two
    /// concurrent callers presenting the same nonce cannot both observe "not seen"
    /// and both proceed; the second `put` is serialised by LMDB and at most one
    /// caller sees `true`.
    pub fn check<N: Nonce>(&self, nonce: &N) -> Result<bool, crate::Error> {
        let now = chrono::Utc::now().timestamp();
        let nonce = nonce.nonce();

        writable_tx(|tx| {
            if Table::RecentNonces.has(tx, nonce)? {
                return Ok(false);
            }
            Table::RecentNonces.put(tx, nonce.as_ref(), now.to_be_bytes())?;
            Ok(true)
        })
    }
}

/// One pass of the cleanup task: deletes nonce entries older than `2 *
/// TOLERATED_AGE`. Wrapped as a separate function so the spawned task can `?`
/// errors and just retry on the next tick instead of unwinding.
fn run_cleanup_pass() -> Result<(), crate::Error> {
    let now = chrono::Utc::now().timestamp();

    writable_tx(|tx| {
        let mut nonces_to_drop: Vec<Vec<u8>> = Vec::new();
        Table::RecentNonces
            .range::<_, [u8; 0]>(..)
            .for_each(tx, |key, value| {
                let bytes: [u8; 8] = value
                    .try_into()
                    .map_err(|_| "bad timestamp from db".to_string())?;
                let then = i64::from_be_bytes(bytes);
                if now - then > 2 * TOLERATED_AGE {
                    nonces_to_drop.push(key.to_vec());
                }
                Ok::<Option<()>, samizdat_common::Error>(None)
            })?;

        for nonce in nonces_to_drop {
            Table::RecentNonces.delete(tx, nonce)?;
        }
        Ok(())
    })
}

impl Nonce for Query {
    fn nonce(&self) -> Hash {
        self.location_riddle.rand
    }
}

impl Nonce for Resolution {
    fn nonce(&self) -> Hash {
        self.location_message_riddle.rand
    }
}

impl Nonce for EditionRequest {
    fn nonce(&self) -> Hash {
        self.key_riddle.rand
    }
}

impl Nonce for EditionAnnouncement {
    fn nonce(&self) -> Hash {
        self.key_riddle.rand
    }
}

impl Nonce for IdentityRequest {
    fn nonce(&self) -> Hash {
        self.identity_riddle.rand
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samizdat_common::db::test_harness::TestDb;
    use samizdat_common::Hash;

    /// Tiny stand-in `Nonce` so tests don't need to construct full `Query`s etc.
    struct N(Hash);
    impl Nonce for N {
        fn nonce(&self) -> Hash {
            self.0
        }
    }

    /// Regression test for H6. A fresh nonce is accepted; the same nonce on a
    /// second call is rejected. Run inside a single `TestDb` so the global LMDB
    /// is initialised once.
    #[test]
    fn check_accepts_then_rejects_replay() {
        TestDb::<crate::db::Table>::with(|| {
            let rr = ReplayResistance;
            let n = N(Hash::rand());
            assert!(rr.check(&n).unwrap(), "fresh nonce should be accepted");
            assert!(!rr.check(&n).unwrap(), "replayed nonce must be rejected");
        });
    }

    /// Regression test for H6. Distinct nonces are independently accepted; the
    /// rejection of one does not leak into the other.
    #[test]
    fn distinct_nonces_are_independent() {
        TestDb::<crate::db::Table>::with(|| {
            let rr = ReplayResistance;
            let a = N(Hash::rand());
            let b = N(Hash::rand());
            assert!(rr.check(&a).unwrap());
            assert!(rr.check(&b).unwrap());
            assert!(!rr.check(&a).unwrap());
            assert!(!rr.check(&b).unwrap());
        });
    }
}
