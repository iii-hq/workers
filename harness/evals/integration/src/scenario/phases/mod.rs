use std::time::Duration;

mod completion;
mod evidence;
mod execution;
mod readiness;

const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TARGET_POLL_INTERVAL: Duration = Duration::from_millis(50);
