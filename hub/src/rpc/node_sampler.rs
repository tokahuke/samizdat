use rand::distributions::Distribution;
use std::collections::{BTreeMap, BinaryHeap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use samizdat_common::heap_entry::HeapEntry;

use super::Node;

#[derive(Debug, Clone)]
struct Normal {
    n: usize,
    x: f64,
    x2: f64,
}

impl Default for Normal {
    fn default() -> Normal {
        // Two pseudo-observations: one of 0.5s and another of 2s.
        Normal {
            n: 2,
            x: 0.0,
            x2: 2f64.ln().powi(2) / 2.0,
        }
    }
}

impl Normal {
    fn observe(&mut self, sample: f64) {
        self.n += 1;
        self.x += sample;
        self.x2 += sample.powi(2);
    }

    fn mean(&self) -> f64 {
        self.x / self.n as f64
    }

    fn var(&self) -> f64 {
        self.x2 / self.n as f64 - self.mean().powi(2)
    }
}

#[derive(Debug)]
struct StatisticsInner {
    requests: f64,
    successes: f64,
    failures: f64,
    latency_success_log: Normal,
}

impl Default for StatisticsInner {
    fn default() -> Self {
        // Ten pseudo-observations: 1 success and 9 failures.
        StatisticsInner {
            requests: 10.0,
            successes: 1.0,
            failures: 9.0,
            latency_success_log: Normal::default(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Statistics(RwLock<StatisticsInner>);

impl Statistics {
    pub fn rand_priority(&self) -> f64 {
        // Use stuff from lock and get rid of it as fast as you can:
        let lock = self.0.read().expect("poisoned");
        let requests = lock.requests;
        let successes = lock.successes;
        let log_normal = lock.latency_success_log.clone();
        drop(lock); // used!

        // Sample a success probability:
        let beta =
            rand_distr::Beta::new(successes + 1., requests + 1.).expect("valid beta distribution");

        let success_prob = beta.sample(&mut rand::thread_rng());

        // Sample a completion time:
        // Normal-inverse gamma prior: lambda = 1, alpha = 0.5, beta = 0, mu0 = 0 (1s)
        let alpha_post = 0.5 * (1. + log_normal.n as f64);
        let beta_post = 0.5
            * log_normal.n as f64
            * (log_normal.var() + log_normal.mean().powi(2) / (log_normal.n as f64 + 1.0));
        let mean_post = log_normal.n as f64 * log_normal.mean() / (log_normal.n as f64 + 1.0);

        // Now that you did the maths, do the sampling:
        let gamma =
            rand_distr::Gamma::new(alpha_post, 1.0 / beta_post).expect("valid gamma distribution");
        let sample_var = 1.0 / gamma.sample(&mut rand::thread_rng());
        let normal = rand_distr::Normal::new(mean_post, sample_var.sqrt())
            .expect("valid normal distribution");
        let sample_latency: f64 = normal.sample(&mut rand::thread_rng()).exp();

        // Focus: maximize _inverse_ latency!
        success_prob / sample_latency
    }

    pub fn start_request(&self) {
        let mut lock = self.0.write().expect("poisoned");
        lock.requests += 1.0;
    }

    pub fn end_request_with_success(&self, latency: Duration) {
        let mut lock = self.0.write().expect("poisoned");
        lock.successes += 1.0;
        lock.latency_success_log
            .observe((latency.as_millis() as f64).max(1.0).ln());
    }

    pub fn end_request_with_failure(&self) {
        let mut lock = self.0.write().expect("poisoned");
        lock.failures += 1.0;
    }
}

pub(super) fn sample(
    sampler: impl PrioritySampler,
    nodes: &BTreeMap<SocketAddr, Arc<Node>>,
) -> impl Iterator<Item = (SocketAddr, Arc<Node>)> {
    let mut queue = BinaryHeap::new();

    // Thompson sampling solution to find the most successful peers.
    for (&node_addr, node) in nodes {
        let priority = (sampler.sample_priority(&node) * 1e6) as i64;

        queue.push(HeapEntry {
            priority,
            content: (node_addr, node.clone()),
        });
    }

    std::iter::from_fn(move || queue.pop().map(|entry| entry.content))
}

pub trait PrioritySampler {
    fn sample_priority(&self, node: &Node) -> f64;
}

#[derive(Debug, Clone, Copy)]
pub struct UniformSampler;

impl PrioritySampler for UniformSampler {
    fn sample_priority(&self, _node: &Node) -> f64 {
        1.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QuerySampler;

impl PrioritySampler for QuerySampler {
    fn sample_priority(&self, node: &Node) -> f64 {
        node.query_statistics.rand_priority()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EditionSampler;

impl PrioritySampler for EditionSampler {
    fn sample_priority(&self, node: &Node) -> f64 {
        node.edition_statistics.rand_priority()
    }
}
