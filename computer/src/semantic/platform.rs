//! The real accessibility providers. A fresh provider per call: the executor
//! builds one per snapshot on its own thread (COM is initialised there on
//! Windows), and dropping it frees xa11y's handle cache.

use std::sync::Arc;

use xa11y_core::Provider;

use super::ProviderFactory;

pub fn factory() -> ProviderFactory {
    Arc::new(|| {
        crate::driver::native::pin_process_dpi();
        #[cfg(target_os = "macos")]
        let provider = xa11y_macos::MacOSProvider::new()?;
        #[cfg(target_os = "windows")]
        let provider = xa11y_windows::WindowsProvider::new()?;
        #[cfg(target_os = "linux")]
        let provider = xa11y_linux::LinuxProvider::new()?;
        Ok(Arc::new(provider) as Arc<dyn Provider>)
    })
}

/// A native Linux session's semantic support as the session bus reports it.
#[cfg(target_os = "linux")]
pub const A11Y_BUS_MISSING: &str = "the AT-SPI accessibility bus is not running in this session";

#[cfg(target_os = "linux")]
pub const A11Y_OFF: &str = "Accessibility is off: GTK apps are readable, but Qt, Firefox, \
     AccessKit (egui/Slint/Bevy) and Chromium/Electron apps expose no tree. To turn it on for \
     this login session run: `gdbus call --session --dest org.a11y.Bus --object-path \
     /org/a11y/bus --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled \
     '<true>'`. Chromium/Electron also need `--force-renderer-accessibility`.";

#[cfg(target_os = "linux")]
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1_500);

/// Set while a probe thread is alive: connect/auth has no timeout of its own,
/// so a stalled bus can park at most this one thread.
#[cfg(target_os = "linux")]
static PROBING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the session bus says about AT-SPI.
#[cfg(target_os = "linux")]
struct BusState {
    /// `org.a11y.Bus.GetAddress` answered.
    address: bool,
    /// `org.a11y.Status.IsEnabled` (unreadable counts as off).
    enabled: bool,
}

/// Read-only: the worker never changes an accessibility setting (spec D3).
#[cfg(target_os = "linux")]
fn read_bus() -> BusState {
    let Ok(c) = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.method_timeout(PROBE_TIMEOUT).build())
    else {
        return BusState {
            address: false,
            enabled: false,
        };
    };
    let address = c
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.a11y.Bus"),
            "GetAddress",
            &(),
        )
        .is_ok();
    let enabled = c
        .call_method(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.a11y.Status", "IsEnabled"),
        )
        .ok()
        .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    BusState { address, enabled }
}

/// `(supported, reason)` for a native Linux session's semantic layer, read
/// once per call with a 1.5 s limit.
#[cfg(target_os = "linux")]
pub async fn a11y_support() -> (bool, Option<String>) {
    support_from(&PROBING, read_bus, PROBE_TIMEOUT).await
}

/// Releases the single-flight flag when the probe thread ends.
#[cfg(target_os = "linux")]
struct ProbeSlot(&'static std::sync::atomic::AtomicBool);

#[cfg(target_os = "linux")]
impl Drop for ProbeSlot {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(target_os = "linux")]
async fn support_from(
    busy: &'static std::sync::atomic::AtomicBool,
    read: impl FnOnce() -> BusState + Send + 'static,
    limit: std::time::Duration,
) -> (bool, Option<String>) {
    use std::sync::atomic::Ordering::{AcqRel, Acquire};
    // Bounded: every D-Bus call (method_timeout) and our wait (`limit`).
    // Connect/auth is not, so `busy` guarantees a probe whose thread is still
    // parked is never joined by another: at most one probe thread exists. A
    // call that finds one running waits for it to end (a healthy probe takes
    // milliseconds), then probes itself, all within the same `limit`.
    let unanswered = || {
        (
            false,
            Some("the AT-SPI accessibility bus did not answer within 1.5 s".to_string()),
        )
    };
    let deadline = tokio::time::Instant::now() + limit;
    // ponytail: polls the flag every 10 ms; a Notify if overlaps get frequent.
    while busy.compare_exchange(false, true, AcqRel, Acquire).is_err() {
        if tokio::time::Instant::now() >= deadline {
            return unanswered();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let task = tokio::task::spawn_blocking(move || {
        let _slot = ProbeSlot(busy);
        read()
    });
    match tokio::time::timeout_at(deadline, task).await {
        Ok(Ok(BusState { address: false, .. })) => (false, Some(A11Y_BUS_MISSING.to_string())),
        Ok(Ok(BusState { enabled: false, .. })) => (true, Some(A11Y_OFF.to_string())),
        Ok(Ok(_)) => (true, None),
        // ponytail: a bus that does not answer in 1.5 s counts as missing.
        _ => unanswered(),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn flag() -> &'static std::sync::atomic::AtomicBool {
        Box::leak(Box::new(std::sync::atomic::AtomicBool::new(false)))
    }

    #[tokio::test]
    async fn the_bus_probe_maps_to_support() {
        let probe = |address, enabled| {
            support_from(
                flag(),
                move || BusState { address, enabled },
                Duration::from_secs(5),
            )
        };
        assert_eq!(
            probe(false, true).await,
            (false, Some(A11Y_BUS_MISSING.to_string()))
        );
        assert_eq!(probe(true, false).await, (true, Some(A11Y_OFF.to_string())));
        assert_eq!(probe(true, true).await, (true, None));
        // The one command that turns it on on every at-spi2-core version.
        assert!(A11Y_OFF.contains("org.a11y.Status IsEnabled '<true>'"));
    }

    #[tokio::test]
    async fn a_silent_bus_does_not_hold_up_the_session() {
        let started = Instant::now();
        let (ok, reason) = support_from(
            flag(),
            || {
                std::thread::sleep(Duration::from_millis(800));
                BusState {
                    address: true,
                    enabled: true,
                }
            },
            Duration::from_millis(50),
        )
        .await;
        assert!(!ok);
        assert!(reason.unwrap().contains("did not answer"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn a_call_overlapping_a_healthy_probe_waits_for_it() {
        let busy = flag();
        let slow = || {
            std::thread::sleep(Duration::from_millis(200));
            BusState {
                address: true,
                enabled: true,
            }
        };
        let quick = || BusState {
            address: true,
            enabled: false,
        };
        let limit = Duration::from_secs(5);
        let (a, b) = tokio::join!(
            support_from(busy, slow, limit),
            support_from(busy, quick, limit)
        );
        assert_eq!(a, (true, None));
        // Its own probe ran once the first ended: no false "did not answer".
        assert_eq!(b, (true, Some(A11Y_OFF.to_string())));
    }

    #[tokio::test]
    async fn a_stalled_probe_is_never_stacked() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        use std::sync::mpsc;
        let busy = flag();
        let runs = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<()>();
        let (r1, r2) = (runs.clone(), runs.clone());
        let limit = Duration::from_millis(50);
        let stalled = move || {
            r1.fetch_add(1, SeqCst);
            let _ = rx.recv();
            BusState {
                address: true,
                enabled: true,
            }
        };
        let second = move || {
            r2.fetch_add(1, SeqCst);
            BusState {
                address: true,
                enabled: true,
            }
        };
        let r3 = runs.clone();
        let (a, b) = tokio::join!(
            support_from(busy, stalled, limit),
            support_from(busy, second, limit)
        );
        assert!(!a.0 && !b.0);
        assert_eq!(runs.load(SeqCst), 1);
        // Released: the next call probes again.
        tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let again = support_from(
            busy,
            move || {
                r3.fetch_add(1, SeqCst);
                BusState {
                    address: true,
                    enabled: true,
                }
            },
            Duration::from_secs(5),
        )
        .await;
        assert_eq!(again, (true, None));
        assert_eq!(runs.load(SeqCst), 2);
    }
}
