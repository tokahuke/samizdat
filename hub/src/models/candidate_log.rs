use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSecondsWithFrac};
use std::time::Duration;

use samizdat_common::rpc::Candidate;

use crate::db::Table;

use super::{Id, Indexable};

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct CandidateLog {
    candidate_log_id: Id,
    /// The id of the query log.
    query_log_id: Id,
    /// The candidate.
    candidate: Candidate,
    /// The outcome of sending the candidate to the client.
    outcome: Option<Result<(), String>>,
    /// The time it took to send the candidate to the client.
    #[serde_as(as = "Option<DurationMilliSecondsWithFrac>")]
    duration: Option<Duration>,
}

impl Indexable for CandidateLog {
    const TABLE: Table = Table::CandidateLog;

    fn id(&self) -> Id {
        self.candidate_log_id
    }
}

impl CandidateLog {
    pub fn new(query_log_id: Id, candidate: Candidate) -> Self {
        Self {
            candidate_log_id: Id::generate(),
            query_log_id,
            candidate,
            outcome: None,
            duration: None,
        }
    }

    pub fn update_with_outcome(&mut self, outcome: Result<(), String>, duration: Duration) {
        self.outcome = Some(outcome);
        self.duration = Some(duration);
    }
}
