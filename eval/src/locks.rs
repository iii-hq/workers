use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::OwnedMutexGuard;

#[derive(Clone, Default)]
pub struct EvalLocks {
    map: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl EvalLocks {
    pub async fn guard(&self, evaluation_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self
                .map
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match map.get(evaluation_id).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    map.insert(evaluation_id.to_string(), Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }
}
