//! Getting work off the connection thread.
//!
//! The SDK spawns every invocation onto the runtime it built for its own
//! `iii-connection` thread — a `current_thread` runtime (`iii.rs:1136`), so
//! every handler in flight shares one thread with the socket that carries
//! the replies those handlers are waiting for. A handler that holds the
//! thread starves the socket: the worker's own calls time out, the queue
//! redelivers the job that caused it, and the redelivery lands on the same
//! saturated thread. Nothing recovers on its own.
//!
//! The ingest pipeline is the one handler heavy enough to reach that point —
//! it measures and trims an evidence bundle for every error span in a trace,
//! four traces at a time — so it runs on the worker's real runtime instead.

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::Handle;

static WORKER: OnceLock<Handle> = OnceLock::new();

/// Remember the worker's own multi-threaded runtime. Called once, from `main`.
pub fn install(handle: Handle) {
    let _ = WORKER.set(handle);
}

/// Run `future` on the worker's runtime rather than wherever it was polled.
pub async fn offload<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    // Tests and `--manifest` never install one; there is nothing to starve.
    let Some(handle) = WORKER.get() else {
        return future.await;
    };
    match handle.spawn(future).await {
        Ok(output) => output,
        Err(joined) => std::panic::resume_unwind(joined.into_panic()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The only installer in the test binary: `WORKER` is process-wide.
    #[test]
    fn the_work_leaves_the_thread_that_asked_for_it() {
        let elsewhere = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("offloaded")
            .enable_all()
            .build()
            .expect("a runtime to hand the work to");
        install(elsewhere.handle().clone());

        let caller = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a caller runtime");
        let ran_on = caller.block_on(offload(async {
            std::thread::current()
                .name()
                .unwrap_or_default()
                .to_string()
        }));

        assert!(
            ran_on.starts_with("offloaded"),
            "the future ran on {ran_on:?}, not on the worker runtime"
        );
    }
}
