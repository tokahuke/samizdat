use crate::Hash;

#[derive(Debug, Clone)]
pub struct ProofOfWork {
    pub information: Hash,
    pub solution: Hash,
}

impl ProofOfWork {
    pub fn new(information: Hash) -> ProofOfWork {
        ProofOfWork {
            information,
            solution: Hash::rand(),
        }
    }

    pub fn proof_hash(&self) -> Hash {
        self.information.rehash(&self.solution)
    }

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

    pub fn improve(&self) -> ProofOfWork {
        let threshold_work = self.work_done();
        let mut improved = self.clone();

        while improved.work_done() <= threshold_work {
            improved.solution = Hash::rand();
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
