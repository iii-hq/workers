//! Copied from judge-typesafe (ponytail: extract into crates/judge-provider when a third provider needs it).
//! Caller-scoped cancellation for in-flight evaluations and model listings.
use judge_contract::ErrorCode;
use std::{
    collections::{hash_map::Entry, HashMap},
    future::pending,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

type CallKey = (String, String);

#[derive(Default)]
pub(crate) struct CancellationRegistry {
    calls: Mutex<HashMap<CallKey, watch::Sender<bool>>>,
}

/// The sole owner of a registration. Cancellation signals but does not remove
/// it, so its ID cannot be reused until the call actually finishes or is dropped.
pub(crate) struct CallGuard {
    registration: Option<(Arc<CancellationRegistry>, CallKey)>,
    signal: Option<watch::Receiver<bool>>,
}

impl CancellationRegistry {
    pub(crate) fn start(
        self: &Arc<Self>,
        caller: Option<&str>,
        id: Option<&str>,
    ) -> Result<CallGuard, ErrorCode> {
        let Some(id) = id else {
            return Ok(CallGuard {
                registration: None,
                signal: None,
            });
        };
        let key = call_key(caller, id)?;
        let mut calls = self.calls.lock().expect("cancellation registry lock");
        match calls.entry(key.clone()) {
            Entry::Occupied(_) => Err(ErrorCode::InvalidRequest),
            Entry::Vacant(entry) => {
                let (sender, signal) = watch::channel(false);
                entry.insert(sender);
                Ok(CallGuard {
                    registration: Some((self.clone(), key)),
                    signal: Some(signal),
                })
            }
        }
    }

    pub(crate) fn cancel(&self, caller: Option<&str>, id: &str) -> Result<bool, ErrorCode> {
        let key = call_key(caller, id)?;
        let calls = self.calls.lock().expect("cancellation registry lock");
        let Some(signal) = calls.get(&key) else {
            return Ok(false);
        };
        signal.send_replace(true);
        Ok(true)
    }
}

impl CallGuard {
    pub(crate) async fn cancelled(&mut self) {
        if let Some(signal) = &mut self.signal {
            // wait_for checks the current value before sleeping. A cancellation
            // sent before the first poll (or between select! calls) stays set.
            if signal.wait_for(|cancelled| *cancelled).await.is_ok() {
                return;
            }
        }
        pending::<()>().await;
    }
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        if let Some((registry, key)) = &self.registration {
            // Only this non-cloneable guard removes the key. No replacement can
            // exist while it is held, including after a cancellation signal.
            registry
                .calls
                .lock()
                .expect("cancellation registry lock")
                .remove(key);
        }
    }
}

fn call_key(caller: Option<&str>, id: &str) -> Result<CallKey, ErrorCode> {
    let caller = caller
        .filter(|caller| !caller.trim().is_empty())
        .ok_or(ErrorCode::InvalidRequest)?;
    if id.trim().is_empty()
        || id.len() > judge_contract::MAX_PROVIDER_REQUEST_ID_BYTES
        || !id.bytes().all(|byte| (b' '..=b'~').contains(&byte))
    {
        return Err(ErrorCode::InvalidRequest);
    }
    Ok((caller.to_owned(), id.to_owned()))
}
