use std::net::IpAddr;

use rocksdb::IteratorMode;
use serde_derive::{Deserialize, Serialize};

use crate::db::{db, Table};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlacklistedIp {
    address: IpAddr,
    since: chrono::DateTime<chrono::Utc>,
}

impl BlacklistedIp {
    pub fn new(address: IpAddr) -> BlacklistedIp {
        BlacklistedIp {
            address,
            since: chrono::Utc::now(),
        }
    }

    pub fn get(address: IpAddr) -> Result<Option<BlacklistedIp>, crate::Error> {
        let maybe_result = db().get_cf(
            Table::BlacklistedIps.get(),
            bincode::serialize(&address).expect("can serialize"),
        )?;

        if let Some(result) = maybe_result {
            Ok(bincode::deserialize(&result)?)
        } else {
            Ok(None)
        }
    }

    pub fn get_all() -> Result<Vec<BlacklistedIp>, crate::Error> {
        db().iterator_cf(Table::BlacklistedIps.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    pub fn insert_with(&self, batch: &mut rocksdb::WriteBatch) {
        batch.put_cf(
            Table::BlacklistedIps.get(),
            bincode::serialize(&self.address).expect("can serialize"),
            bincode::serialize(&self).expect("can serialize"),
        )
    }

    pub fn insert(&self) -> Result<(), crate::Error> {
        let mut batch = rocksdb::WriteBatch::default();
        self.insert_with(&mut batch);
        db().write(batch)?;

        Ok(())
    }
}
