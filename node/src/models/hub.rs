use serde_derive::{Deserialize, Serialize};

use samizdat_common::{
    address::AddrResolutionMode,
    db::{Droppable, Table as _, TxHandle, WritableTx},
};

use crate::db::Table;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hub {
    pub address: String,
    pub resolution_mode: AddrResolutionMode,
}

impl Droppable for Hub {
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        let address = self.address.to_string();
        Table::Hubs.delete(tx, &address);
        tokio::spawn(async move { crate::hubs().remove(&address).await });
        Ok(())
    }
}

impl Hub {
    /// Inserts the current identity in the database using the supplied [`WriteBatch`].
    pub fn insert(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        let hub = self.clone();
        tokio::spawn(async move { crate::hubs().insert(hub).await });

        Table::Hubs.put(
            tx,
            self.address.as_str(),
            bincode::serialize(&self).expect("can serialize"),
        );

        Ok(())
    }

    /// Lists all hubs currently in the database.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<Hub>, crate::Error> {
        Table::Hubs
            .range(..)
            .collect(tx, |_, value| Ok(bincode::deserialize(value)?))
    }

    pub fn get<Tx: TxHandle>(tx: &Tx, address: &str) -> Result<Option<Hub>, crate::Error> {
        Ok(Table::Hubs
            .get(tx, address, |k| bincode::deserialize(k))
            .transpose()?)
    }
}
