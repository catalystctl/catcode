use crate::config::Config;
use crate::tools::{dispatch_bulk_inner, Outcome};
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde_json::Value;

const WAVE_CONCURRENCY: usize = 8;

/// Run an already-authorized read wave concurrently while retaining model
/// order in the result vector. Mutating calls never reach this function: the
/// caller admits only tools classified `ParallelSafety::Safe`.
pub async fn execute_parallel_wave(calls: &[(String, Value)], cfg: &Config) -> Vec<Outcome> {
    if calls.is_empty() {
        return Vec::new();
    }
    if calls.len() == 1 {
        return vec![dispatch_bulk_inner(&calls[0].0, &calls[0].1, cfg).await];
    }
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(WAVE_CONCURRENCY));
    let mut pending = FuturesUnordered::new();
    for (index, (name, args)) in calls.iter().enumerate() {
        let semaphore = semaphore.clone();
        let cfg = cfg.clone();
        let name = name.clone();
        let args = args.clone();
        pending.push(async move {
            let _permit = semaphore.acquire().await.ok();
            (index, dispatch_bulk_inner(&name, &args, &cfg).await)
        });
    }
    let mut outcomes = Vec::with_capacity(calls.len());
    while let Some(pair) = pending.next().await {
        outcomes.push(pair);
    }
    outcomes.sort_by_key(|(index, _)| *index);
    outcomes.into_iter().map(|(_, outcome)| outcome).collect()
}
