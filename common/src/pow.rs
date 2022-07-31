//! A proof-of-work implementation using [`crate::Hash`].

use serde_derive::{Deserialize, Serialize};

use crate::Hash;

/// A proof-of-work implementation using [`crate::Hash`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfWork {
    /// The information (challenge) to be proved.
    pub information: Hash,
    /// The hash that, combined with the information using [`Hash::rehash`], yields the
    /// proof hash.
    pub solution: Hash,
}

impl ProofOfWork {
    /// Creates a new proof of work for a given challenge hash. The solution is
    /// initialized to be a random hash.
    pub fn new(information: Hash) -> ProofOfWork {
        ProofOfWork {
            information,
            solution: Hash::rand(),
        }
    }

    /// Calculates the proof hash for this proof of work.
    pub fn proof_hash(&self) -> Hash {
        self.information.rehash(&self.solution)
    }

    /// A metric of how much work was done to calculate the current hash. It is expected
    /// that this number corresponds, on average, to the number of random solutions
    /// generated before arriving to this result.
    pub fn work_done(&self) -> f64 {
        let proof = self.proof_hash();
        let mut inv_work_done = 0.0;
        let mut base = 1.0 / 256.0;

        for byte in proof.0 {
            inv_work_done += base * byte as f64;
            base /= 256.0;
        }

        1.0 / inv_work_done - 1.0
    }

    /// Creates random hashes until the work done metric is improved. On the first
    /// improvement, returns the improved solution.
    pub fn improve(&self) -> ProofOfWork {
        let mut rng = crate::csprng();
        let threshold_work = self.work_done();
        let mut improved = self.clone();

        while improved.work_done() <= threshold_work {
            improved.solution = Hash::rand_with(&mut rng);
        }

        improved
    }
}

#[test]
fn test_pow() {
    let information = Hash::rand();
    let pow = ProofOfWork::new(information);
    println!("Proof hash: {}", pow.proof_hash());
    println!("Work done: {}", pow.work_done());
}

#[test]
fn test_pow_improve() {
    let information = Hash::rand();
    let mut pow = ProofOfWork::new(information);

    for _ in 0..20 {
        pow = pow.improve();
        println!("Proof hash: {}", pow.proof_hash());
        println!("Work done: {}", pow.work_done());
    }
}
