use iii_sdk::IIIClient;
use std::sync::Arc;

use crate::configuration::AppState;

pub mod accounts;
pub mod attachment;
pub mod flag;
pub mod get;
pub mod list;
pub mod move_;
pub mod search;
pub mod send;

pub const REGISTERED_FN_COUNT: usize = 8;

pub fn register_all(iii: &Arc<IIIClient>, state: &AppState) {
    send::register(iii, &state.cell);
    accounts::register(iii, &state.cell);
    list::register(iii, &state.pool);
    get::register(iii, &state.pool);
    search::register(iii, &state.pool);
    flag::register(iii, &state.pool);
    move_::register(iii, &state.pool);
    attachment::register(iii, &state.pool);
}
