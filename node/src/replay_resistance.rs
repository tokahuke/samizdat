use std::convert::TryInto;
use tokio::time::{interval, Duration};

use samizdat_common::rpc::Query;

use crate::db;
use crate::db::Table;

/// 10min allows for some sloppy clocks out there.
const TOLLERATED_AGE: i64 = 600;

pub struct ReplayResistance;

impl ReplayResistance {
    pub fn new() -> ReplayResistance {
        // Cleanup old nonces. Made deliberately infrequent
        tokio::spawn(async move {
            // Twice would suffice, but thrice is certainty.
            let mut interval = interval(Duration::from_secs(TOLLERATED_AGE as u64 * 3));

            loop {
                let now = chrono::Utc::now().timestamp();

                for (key, val) in
                    db().iterator_cf(Table::RecentNonces.get(), rocksdb::IteratorMode::Start)
                {
                    let then =
                        i64::from_be_bytes((&*val).try_into().expect("bad timestamp from db"));
                    if now - then > 2 * TOLLERATED_AGE {
                        // Errors here are leaky, but not a security risk.
                        db().delete_cf(Table::RecentNonces.get(), key).ok();
                    }
                }

                interval.tick().await;
            }
        });

        ReplayResistance
    }

    /// Mutability ensures sequential checking of queries, which prevents TOCTOU
    /// when I check against the DB (kinda... must ensure *singleton*).
    pub fn check(&mut self, query: &Query) -> Result<bool, crate::Error> {
        // Is timestamp recent? (timestamp is guaranteed to be geerated by the
        // client because hashes!)
        let now = chrono::Utc::now().timestamp();
        let then = query.content_riddle.timestamp;
        if (now - then).abs() > TOLLERATED_AGE {
            return Ok(false);
        }

        // Have I already seen this none before?
        if db()
            .get_cf(Table::RecentNonces.get(), &query.content_riddle.rand)?
            .is_some()
        {
            return Ok(false);
        }

        db().put_cf(
            Table::RecentNonces.get(),
            &query.content_riddle.rand,
            &query.content_riddle.timestamp.to_be_bytes(),
        )?;

        Ok(true)
    }
}
