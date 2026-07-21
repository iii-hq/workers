//! Process spawning and teardown for isolated integration stacks.
//!
//! Every spawned process becomes the leader of a new process group. Signals
//! are sent to the group rather than only to the direct child, so helper
//! processes cannot survive a scenario teardown or a partially failed boot.

mod child;
mod spec;
mod supervisor;

pub use child::SupervisedChild;
pub use spec::ProcessSpec;
pub use supervisor::{
    EarlyExit, ProcessSupervisor, TeardownIssue, TeardownReport, DEFAULT_TEARDOWN_BUDGET,
};

#[cfg(test)]
mod tests;
