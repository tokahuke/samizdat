//! Periodic snapshot of the per-node sampler statistics, kept for
//! offline analysis of hub-side routing quality.

use serde_derive::{Deserialize, Serialize};

use crate::{
    db::Table,
    models::{Id, Indexable},
    rpc::node_sampler::StatisticsSnapshot,
};

/// One snapshot row: the sampler's full per-node state at a point in
/// time.
#[derive(Debug, Serialize, Deserialize)]
pub struct StatisticsLog {
    /// Primary key.
    id: Id,
    /// Sampler statistics captured at the snapshot moment.
    statistics: StatisticsSnapshot,
}

impl Indexable for StatisticsLog {
    const TABLE: Table = Table::StatisticsLog;

    fn id(&self) -> Id {
        self.id
    }
}

impl StatisticsLog {
    pub fn new(id: Id, statistics: StatisticsSnapshot) -> Self {
        Self { id, statistics }
    }

    pub fn statistics(&self) -> &StatisticsSnapshot {
        &self.statistics
    }
}
