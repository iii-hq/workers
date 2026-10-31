//! Boot sequence: the scheduler, `cron::on-tick` (whose observers retain the
//! scheduler's jobs, see [`crate::on_tick`]) and the console UI.

use std::sync::Arc;

use iii_sdk::runtime::FunctionRef;
use iii_sdk::IIIClient;

use crate::on_tick::{self, OnTickCaller};
use crate::scheduler::Scheduler;

/// A running worker: the scheduler its observers retain jobs in, and
/// `cron::on-tick`.
pub struct BootHandle {
    pub scheduler: Arc<Scheduler>,
    /// `cron::on-tick`; each of its observers retains a job in `scheduler`.
    pub on_tick: FunctionRef,
    iii: Arc<IIIClient>,
}

impl BootHandle {
    /// Unregisters `cron::on-tick`, clears its hooks and stops every job.
    /// The engine holds the function's observers until it is registered
    /// again, by a restarted worker or a standby instance.
    pub async fn shutdown(&self) {
        self.on_tick.unregister();
        on_tick::clear_hooks(&self.iii);
        self.scheduler.shutdown().await;
    }
}

/// Resolves with the error that stopped `iii` for good: the engine refused
/// this worker's registration. The engine admits one live worker per worker
/// name and namespace, so a second instance under the same name gets
/// `WORKER_NAMESPACE_CONFLICT` and never registers `cron::on-tick`; a standby
/// instance connects under its own name (`III_WORKER_NAME`). The SDK does not
/// reconnect after this error, so the process should exit.
pub async fn refused(iii: &IIIClient) -> iii_sdk::Error {
    loop {
        if let Some(err) = iii.fatal_error() {
            return err;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

/// Registers this worker's function ([`on_tick::ON_TICK_FN`]) over a fresh
/// scheduler, and its console UI.
pub fn start(iii: Arc<IIIClient>) -> BootHandle {
    let scheduler = Arc::new(Scheduler::new(Arc::new(OnTickCaller::new(iii.clone()))));
    let on_tick = on_tick::register(&iii, scheduler.clone());

    crate::ui::register(&iii);

    BootHandle {
        scheduler,
        on_tick,
        iii,
    }
}
