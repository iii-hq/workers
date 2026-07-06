//! Per-repository in-process serialization. Every mutating handler and every
//! land step guards the repo key so read-modify-write paths never interleave
//! within one process. This is also the backstop when the operator forgot
//! the FIFO `queue_configs` block for the land queue.
//!
//! Single-process correctness only: iii-state exposes no compare-and-set, so
//! a multi-instance deployment must shard by `repo_key` (same stance as the
//! `workflow` worker).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Default)]
pub struct RepoLocks {
    map: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl RepoLocks {
    /// Acquire the lock for `repo_key`, creating it on first use. Storing
    /// `Weak` lets a quiet repo's mutex deallocate instead of pinning the
    /// map forever.
    pub async fn guard(&self, repo_key: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
            match map.get(repo_key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    // A fresh key is a natural point to sweep tombstones left
                    // by quiet repos whose mutex has since deallocated, so the
                    // map does not grow without bound.
                    map.retain(|_, weak| weak.strong_count() > 0);
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(repo_key.to_string(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_key_serializes_different_keys_do_not() {
        let locks = RepoLocks::default();
        let g1 = locks.guard("a").await;
        let g2 = locks.guard("b").await;
        drop(g2);
        let locks2 = locks.clone();
        let handle = tokio::spawn(async move {
            let _g = locks2.guard("a").await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!handle.is_finished(), "second guard on same key must wait");
        drop(g1);
        handle.await.unwrap();
    }
}
