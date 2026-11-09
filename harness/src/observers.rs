//! Observers the harness keeps through its connection's absences.
//!
//! The SDK replays this worker's observers on every reconnect, and a
//! `Reattach` within the engine's `observers.observer_grace_ms` keeps them.
//! Away longer than that, the engine disposes them (`OBSERVER_DISCONNECTED`)
//! and, within `observers.disposed_retention_ms`, answers the replay with that
//! disposal: by the observe-v2 contract the observer stays disposed and its
//! owner observes again (acceptance E17). Until it does, nothing the observer
//! watched reaches the harness, so every observer the harness means to keep
//! is observed again here.

use std::sync::{Arc, Mutex};

use iii_sdk::{ObserverHandle, ObserverState};

/// Why the engine disposes an observer whose worker did not come back within
/// the grace period.
pub const OBSERVER_DISCONNECTED: &str = "OBSERVER_DISCONNECTED";

/// Whether the engine disposed `handle`'s observer because this worker was
/// away past the grace period.
pub fn disposed_while_away(handle: &ObserverHandle) -> bool {
    handle.state() == ObserverState::Disposed
        && handle
            .reason()
            .is_some_and(|reason| reason.code == OBSERVER_DISCONNECTED)
}

/// Run `again` once, inside the runtime that called this, when the engine
/// disposes `handle`'s observer because this worker was away past the grace
/// period. Any other end of the observer (a dispose, an expiry, a refusal)
/// runs nothing.
pub fn when_disposed_while_away(handle: &ObserverHandle, again: impl FnOnce() + Send + 'static) {
    let runtime = tokio::runtime::Handle::try_current().ok();
    let again = Mutex::new(Some(again));
    // The listener lives as long as the observer; the token only unsubscribes.
    let _ = handle.on_state_change(move |state, reason| {
        if *state != ObserverState::Disposed
            || reason.map(|reason| reason.code.as_str()) != Some(OBSERVER_DISCONNECTED)
        {
            return;
        }
        let Some(again) = again.lock().unwrap_or_else(|p| p.into_inner()).take() else {
            return;
        };
        // Listeners run on the SDK's listener thread, outside any runtime.
        let _entered = runtime.as_ref().map(|runtime| runtime.enter());
        again();
    });
}

/// The observer `observe` creates, observed again with `observe` each time the
/// engine disposes it while the harness was away. Dropping it keeps observing;
/// [`Kept::dispose`] stops.
pub struct Kept(Slot);

type Slot = Arc<Mutex<Option<ObserverHandle>>>;
type Observe = Arc<dyn Fn() -> ObserverHandle + Send + Sync>;

pub fn keep(observe: impl Fn() -> ObserverHandle + Send + Sync + 'static) -> Kept {
    let slot: Slot = Arc::default();
    let observe: Observe = Arc::new(observe);
    let mut current = slot.lock().unwrap_or_else(|p| p.into_inner());
    *current = Some(observe_into(&slot, observe));
    drop(current);
    Kept(slot)
}

/// Observe with `observe`; when the engine disposes that observer while the
/// harness was away, observe again into `slot`, unless it was disposed.
fn observe_into(slot: &Slot, observe: Observe) -> ObserverHandle {
    let handle = observe();
    let slot = slot.clone();
    when_disposed_while_away(&handle, move || {
        let mut current = slot.lock().unwrap_or_else(|p| p.into_inner());
        if current.is_some() {
            let handle = observe_into(&slot, observe);
            tracing::info!(observer = %handle.id(), "observed again: the engine disposed the observer while the harness was away");
            *current = Some(handle);
        }
    });
    handle
}

impl Kept {
    /// Stop observing again and dispose the current observer.
    pub async fn dispose(&self) -> Result<(), iii_sdk::Error> {
        let handle = self.0.lock().unwrap_or_else(|p| p.into_inner()).take();
        match handle {
            Some(handle) => handle.dispose().await,
            None => Ok(()),
        }
    }
}
