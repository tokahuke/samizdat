use serde_derive::{Deserialize, Serialize};
use std::net::IpAddr;

use samizdat_common::db::{Table as _, TxHandle, WritableTx};

use crate::db::Table;

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

    pub fn get<Tx: TxHandle>(
        tx: &Tx,
        address: IpAddr,
    ) -> Result<Option<BlacklistedIp>, crate::Error> {
        Table::BlacklistedIps.get(
            tx,
            bincode::serialize(&address).expect("can serialize"),
            |result| Ok(bincode::deserialize(result)?),
        )
    }

    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<BlacklistedIp>, crate::Error> {
        // One corrupt/legacy row used to crash the HTTP handler. Propagate instead so
        // the operator gets a 500 with the error message and a deserializable subset
        // can still be reasoned about.
        let collected: Result<Vec<BlacklistedIp>, crate::Error> = Table::BlacklistedIps
            .range::<_, [u8; 0]>(..)
            .collect(tx, |_, value| {
                Ok::<BlacklistedIp, crate::Error>(bincode::deserialize(value)?)
            })?;
        collected
    }

    pub fn insert(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        Table::BlacklistedIps.put(
            tx,
            bincode::serialize(&self.address).expect("can serialize"),
            bincode::serialize(&self).expect("can serialize"),
        )
    }
}
