//! Bounded, batching writer for query logs.
//!
//! The resolver used to `tokio::spawn` one task per query, each opening its own
//! transaction. Two problems compounded: SQLite serialises writers so the
//! commits queued up behind each other, and the spawning was unbounded, so a
//! query rate above the commit rate grew the pending task set without limit.
//!
//! This writer replaces that with a bounded channel drained by a single task
//! that groups rows into one transaction. Query logs are observability data, so
//! when the queue is full the newest entry is dropped and counted rather than
//! applying backpressure to DNS resolution.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::database::Database;
use super::models::CreateQueryLog;

/// Queue capacity. Deep enough to absorb a burst, small enough that a stalled
/// writer cannot hold an unbounded amount of memory.
const QUEUE_CAPACITY: usize = 4096;

/// Maximum rows per transaction.
const MAX_BATCH: usize = 256;

/// How long a partial batch waits for more rows before being committed.
const LINGER: Duration = Duration::from_millis(200);

/// How often to report accumulated drops.
const DROP_REPORT_INTERVAL: u64 = 1000;

/// Handle used by the query path to enqueue log entries.
pub struct QueryLogWriter {
    sender: mpsc::Sender<CreateQueryLog>,
    /// Entries discarded because the queue was full.
    dropped: AtomicU64,
}

impl QueryLogWriter {
    /// Start the background writer and return the handle for enqueueing.
    pub fn start(db: Arc<Database>) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);

        tokio::spawn(run_writer(db, receiver));

        Arc::new(Self {
            sender,
            dropped: AtomicU64::new(0),
        })
    }

    /// Enqueue a log entry, dropping it if the queue is full.
    ///
    /// Never blocks and never fails: resolution must not slow down or fail
    /// because logging cannot keep up.
    pub fn enqueue(&self, log: CreateQueryLog) {
        if self.sender.try_send(log).is_err() {
            self.note_drop();
        }
    }

    /// Number of entries dropped so far.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Count a drop, reporting periodically so the loss is visible without
    /// emitting a line per dropped entry.
    fn note_drop(&self) {
        let total = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if total % DROP_REPORT_INTERVAL == 1 {
            warn!(
                "Query log queue full, dropped {} entries so far (DNS resolution is unaffected)",
                total
            );
        }
    }
}

/// Drain the queue, committing rows in batches until the channel closes.
async fn run_writer(db: Arc<Database>, mut receiver: mpsc::Receiver<CreateQueryLog>) {
    let mut batch: Vec<CreateQueryLog> = Vec::with_capacity(MAX_BATCH);

    while let Some(first) = receiver.recv().await {
        batch.push(first);
        fill_batch(&mut receiver, &mut batch).await;
        flush_batch(&db, &mut batch).await;
    }

    // Channel closed: commit whatever is left so shutdown does not lose rows.
    flush_batch(&db, &mut batch).await;
    debug!("Query log writer stopped");
}

/// Collect additional rows until the batch is full or the linger window ends.
async fn fill_batch(
    receiver: &mut mpsc::Receiver<CreateQueryLog>,
    batch: &mut Vec<CreateQueryLog>,
) {
    let deadline = tokio::time::Instant::now() + LINGER;

    while batch.len() < MAX_BATCH {
        match tokio::time::timeout_at(deadline, receiver.recv()).await {
            Ok(Some(log)) => batch.push(log),
            // Channel closed, or the linger window expired.
            Ok(None) | Err(_) => break,
        }
    }
}

/// Commit the batch, clearing it either way.
///
/// A failed batch is reported and discarded: retrying would stall the queue and
/// grow the backlog behind observability data.
async fn flush_batch(db: &Database, batch: &mut Vec<CreateQueryLog>) {
    if batch.is_empty() {
        return;
    }

    if let Err(e) = db.query_logs().create_batch(batch).await {
        warn!("Failed to write {} query log entries: {}", batch.len(), e);
    }

    batch.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_log(name: &str) -> CreateQueryLog {
        CreateQueryLog {
            client_ip: "127.0.0.1".to_string(),
            query_name: name.to_string(),
            query_type: "A".to_string(),
            response_code: Some("NOERROR".to_string()),
            response_time: Some(1),
            cache_hit: false,
            upstream_used: None,
        }
    }

    /// A full queue must drop entries rather than block the caller: the DNS
    /// query path cannot wait on logging.
    #[tokio::test]
    async fn test_enqueue_drops_when_queue_full() {
        let (sender, receiver) = mpsc::channel(1);
        let writer = QueryLogWriter {
            sender,
            dropped: AtomicU64::new(0),
        };

        // Nothing drains the receiver, so only the first entry fits.
        writer.enqueue(sample_log("first.example.com"));
        writer.enqueue(sample_log("second.example.com"));
        writer.enqueue(sample_log("third.example.com"));

        assert_eq!(writer.dropped_count(), 2);
        drop(receiver);
    }

    /// The linger window must end a partial batch, otherwise a low query rate
    /// would leave rows uncommitted indefinitely.
    #[tokio::test]
    async fn test_fill_batch_stops_at_linger_deadline() {
        let (sender, mut receiver) = mpsc::channel(8);
        sender.send(sample_log("a.example.com")).await.unwrap();

        let mut batch = vec![receiver.recv().await.unwrap()];
        fill_batch(&mut receiver, &mut batch).await;

        // Only the one available row; the call returned instead of waiting for
        // MAX_BATCH rows that will never arrive.
        assert_eq!(batch.len(), 1);
        drop(sender);
    }

    /// A closed channel must end the batch immediately so shutdown can flush.
    #[tokio::test]
    async fn test_fill_batch_stops_when_channel_closed() {
        let (sender, mut receiver) = mpsc::channel(8);
        sender.send(sample_log("a.example.com")).await.unwrap();
        sender.send(sample_log("b.example.com")).await.unwrap();
        drop(sender);

        let mut batch = vec![receiver.recv().await.unwrap()];
        fill_batch(&mut receiver, &mut batch).await;

        assert_eq!(batch.len(), 2);
    }
}
