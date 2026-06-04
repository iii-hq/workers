use iii_sdk::III;
use std::sync::Arc;

pub mod accounts;
pub mod attachment;
pub mod flag;
pub mod get;
pub mod list;
pub mod move_;
pub mod search;
pub mod send;

pub const REGISTERED_FN_COUNT: usize = 8;

pub fn register_all(
    iii: &Arc<III>,
    cfg: &Arc<crate::config::WorkerConfig>,
    pool: &Arc<crate::provider::imap::ImapPool>,
) {
    send::register(iii, cfg);
    accounts::register(iii, cfg);
    list::register(iii, pool);
    get::register(iii, pool);
    search::register(iii, pool);
    flag::register(iii, pool);
    move_::register(iii, pool);
    attachment::register(iii, pool);
}
