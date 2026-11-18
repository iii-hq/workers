//! A journal append that fails must leave the file-backed store unchanged:
//! the caller gets the error, live queues and counters stay put, and a reopen
//! finds exactly what the live store still reports.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use iii_queue::store::{FileStore, QueueStore};
use serde_json::json;
use uuid::Uuid;

const TOPIC: &str = "probe";

/// Replaces the journal with a directory so every append fails with EISDIR,
/// keeping the original bytes aside to prove nothing was written.
struct BrokenJournal {
    journal: PathBuf,
    saved: PathBuf,
    before: Vec<u8>,
}

impl BrokenJournal {
    fn new(dir: &Path) -> Self {
        let journal = dir.join("queue_store.journal");
        let saved = dir.join("journal.saved");
        let before = fs::read(&journal).unwrap();
        fs::rename(&journal, &saved).unwrap();
        fs::create_dir(&journal).unwrap();
        Self {
            journal,
            saved,
            before,
        }
    }

    fn restore(self) {
        assert_eq!(
            fs::read(&self.saved).unwrap(),
            self.before,
            "journal must be unchanged"
        );
        fs::remove_dir(&self.journal).unwrap();
        fs::rename(&self.saved, &self.journal).unwrap();
    }
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("queue-journal-failure-{}", Uuid::new_v4()))
}

async fn open(dir: &Path) -> FileStore {
    FileStore::open(dir, 60_000).await.unwrap()
}

#[tokio::test]
async fn failed_ack_is_reported_and_keeps_the_job_in_flight() {
    let dir = temp_dir();
    let store = open(&dir).await;
    let id = store.enqueue(TOPIC, json!({"job": 1})).await.unwrap();
    assert_eq!(store.dequeue(TOPIC).await.unwrap().unwrap().id, id);

    let broken = BrokenJournal::new(&dir);
    assert!(
        store.ack(TOPIC, &id).await.is_err(),
        "a lost ACK must be reported"
    );
    assert_eq!(store.topic_stats(TOPIC).await.delivered, 0);
    broken.restore();

    // Live and persisted state agree: the job was never acknowledged.
    drop(store);
    let reopened = open(&dir).await;
    assert_eq!(reopened.dequeue(TOPIC).await.unwrap().unwrap().id, id);
    drop(reopened);
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn ack_retried_after_a_failed_append_is_durable() {
    let dir = temp_dir();
    let store = open(&dir).await;
    let id = store.enqueue(TOPIC, json!({"job": 1})).await.unwrap();
    store.dequeue(TOPIC).await.unwrap().unwrap();

    let broken = BrokenJournal::new(&dir);
    assert!(store.ack(TOPIC, &id).await.is_err());
    broken.restore();
    store.ack(TOPIC, &id).await.unwrap();
    assert_eq!(store.topic_stats(TOPIC).await.delivered, 1);

    drop(store);
    let reopened = open(&dir).await;
    assert!(reopened.dequeue(TOPIC).await.unwrap().is_none());
    drop(reopened);
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn failed_dequeue_hands_out_no_job() {
    let dir = temp_dir();
    let store = open(&dir).await;
    let id = store.enqueue(TOPIC, json!({"job": 1})).await.unwrap();

    let broken = BrokenJournal::new(&dir);
    assert!(store.dequeue(TOPIC).await.is_err());
    assert_eq!(store.topic_stats(TOPIC).await.depth, 1);
    broken.restore();

    assert_eq!(store.dequeue(TOPIC).await.unwrap().unwrap().id, id);
    drop(store);
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn failed_nack_does_not_dead_letter_the_job() {
    let dir = temp_dir();
    let store = open(&dir).await;
    store.enqueue(TOPIC, json!({"job": 1})).await.unwrap();
    let job = store.dequeue(TOPIC).await.unwrap().unwrap();

    let broken = BrokenJournal::new(&dir);
    assert!(store.nack(TOPIC, job.clone(), 1, 1).await.is_err());
    let stats = store.topic_stats(TOPIC).await;
    assert_eq!((stats.failed, stats.dlq_depth, stats.depth), (0, 0, 0));
    broken.restore();

    store.nack(TOPIC, job, 1, 1).await.unwrap();
    assert_eq!(store.topic_stats(TOPIC).await.dlq_depth, 1);
    drop(store);
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn failed_dlq_operations_leave_the_dlq_intact() {
    let dir = temp_dir();
    let store = open(&dir).await;
    let id = store.enqueue(TOPIC, json!({"job": 1})).await.unwrap();
    let job = store.dequeue(TOPIC).await.unwrap().unwrap();
    store.nack(TOPIC, job, 1, 1).await.unwrap();

    let broken = BrokenJournal::new(&dir);
    assert!(store.redrive_dlq(TOPIC).await.is_err());
    assert!(store.redrive_dlq_message(TOPIC, &id).await.is_err());
    assert!(store.discard_dlq_message(TOPIC, &id).await.is_err());
    let stats = store.topic_stats(TOPIC).await;
    assert_eq!((stats.dlq_depth, stats.depth), (1, 0));
    broken.restore();

    drop(store);
    let reopened = open(&dir).await;
    assert_eq!(reopened.dlq_messages(TOPIC, 10).await[0].id, id);
    drop(reopened);
    fs::remove_dir_all(&dir).unwrap();
}
