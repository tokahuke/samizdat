use rand::distributions::Distribution;
use std::sync::RwLock;
use std::time::Duration;

#[derive(Debug, Default)]
struct StatisticsInner {
    requests: usize,
    successes: usize,
    errors: usize,
    total_latency_success: usize,
    total_latency_error: usize,
}

#[derive(Debug, Default)]
pub struct Statistics(RwLock<StatisticsInner>);

impl Statistics {
    pub fn rand_priority(&self) -> f64 {
        let lock = self.0.read().expect("poisoned");
        let beta = rand_distr::Beta::new(lock.successes as f64 + 1., lock.requests as f64 + 1.)
            .expect("valid beta distribution");

        beta.sample(&mut rand::thread_rng())
    }

    pub fn start_request(&self) {
        let mut lock = self.0.write().expect("poisoned");
        lock.requests += 1;
    }

    pub fn end_request_with_success(&self, latency: Duration) {
        let mut lock = self.0.write().expect("poisoned");
        lock.successes += 1;
        lock.total_latency_success += latency.as_millis() as usize;
    }

    pub fn end_request_with_error(&self, latency: Duration) {
        let mut lock = self.0.write().expect("poisoned");
        lock.errors += 1;
        lock.total_latency_error += latency.as_millis() as usize;
    }
}
