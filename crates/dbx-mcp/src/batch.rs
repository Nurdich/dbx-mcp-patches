//! Sequential / parallel batch runners for connection ranges.

use std::future::Future;
use std::sync::Arc;

use dbx_core::models::connection::ConnectionConfig;
use tokio::sync::Semaphore;

use crate::list_index::DEFAULT_PARALLEL_CONCURRENCY;

#[derive(Debug)]
pub enum BatchItem<T> {
    Ok { index: usize, value: T },
    Err { index: usize, name: String, code: String, message: String },
    Skipped { index: usize, name: String, code: String, message: String },
}

impl<T> BatchItem<T> {
    pub fn index(&self) -> usize {
        match self {
            Self::Ok { index, .. } | Self::Err { index, .. } | Self::Skipped { index, .. } => *index,
        }
    }
}

pub fn batch_heading(config: &ConnectionConfig, index: usize, total: usize) -> String {
    if total <= 1 {
        String::new()
    } else {
        format!("## #{index} {}\n\n", config.name)
    }
}

pub fn batch_summary(total: usize, ok: usize, skipped: usize, failures: usize) -> String {
    format!("---\nBatch summary: {total} connection(s), {ok} ok, {skipped} skipped, {failures} failed.")
}

/// Run `worker` for each connection. `parallel = None` → sequential; `Some(n)` → concurrency n (0 → default 15).
pub async fn run_connection_batch<T, F, Fut>(
    configs: &[ConnectionConfig],
    parallel: Option<usize>,
    worker: F,
) -> Vec<BatchItem<T>>
where
    T: Send + 'static,
    F: Fn(ConnectionConfig, usize) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, (String, String, bool)>> + Send + 'static,
{
    // Err tuple: (code, message, skipped)
    if configs.is_empty() {
        return Vec::new();
    }
    if configs.len() == 1 || parallel.is_none() {
        let mut results = Vec::with_capacity(configs.len());
        for (index, config) in configs.iter().enumerate() {
            results.push(run_one(config.clone(), index, &worker).await);
        }
        return results;
    }

    let concurrency = parallel.unwrap_or(DEFAULT_PARALLEL_CONCURRENCY).max(1).min(configs.len());
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let worker = Arc::new(worker);
    let mut handles = Vec::with_capacity(configs.len());
    for (index, config) in configs.iter().cloned().enumerate() {
        let semaphore = Arc::clone(&semaphore);
        let worker = Arc::clone(&worker);
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.expect("semaphore");
            run_one(config, index, worker.as_ref()).await
        }));
    }
    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(item) => results.push(item),
            Err(join_error) => results.push(BatchItem::Err {
                index: results.len(),
                name: String::new(),
                code: "JOIN_ERROR".into(),
                message: join_error.to_string(),
            }),
        }
    }
    results.sort_by_key(BatchItem::index);
    results
}

async fn run_one<T, F, Fut>(config: ConnectionConfig, index: usize, worker: &F) -> BatchItem<T>
where
    F: Fn(ConnectionConfig, usize) -> Fut,
    Fut: Future<Output = Result<T, (String, String, bool)>>,
{
    let name = config.name.clone();
    match worker(config, index).await {
        Ok(value) => BatchItem::Ok { index, value },
        Err((code, message, true)) => BatchItem::Skipped { index, name, code, message },
        Err((code, message, false)) => BatchItem::Err { index, name, code, message },
    }
}

pub fn count_batch<T>(items: &[BatchItem<T>]) -> (usize, usize, usize) {
    let mut ok = 0usize;
    let mut skipped = 0usize;
    let mut failures = 0usize;
    for item in items {
        match item {
            BatchItem::Ok { .. } => ok += 1,
            BatchItem::Skipped { .. } => skipped += 1,
            BatchItem::Err { .. } => failures += 1,
        }
    }
    (ok, skipped, failures)
}
