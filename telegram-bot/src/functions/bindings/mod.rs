pub mod message_added;
pub mod message_updated;
pub mod pending_created;
pub mod pending_resolved;
pub mod status_changed;
pub mod turn_completed;

use std::sync::Arc;

use iii_sdk::III;

use crate::deps::Deps;

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    message_added::register(iii, deps);
    message_updated::register(iii, deps);
    status_changed::register(iii, deps);
    turn_completed::register(iii, deps);
    pending_created::register(iii, deps);
    pending_resolved::register(iii, deps);
}
