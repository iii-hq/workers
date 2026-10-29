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
