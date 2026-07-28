//! `database::row-changed` — the worker announcing its own writes.
//!
//! The database is often the state machine of a run: workers write rows and
//! something else needs to know. Until now nothing emitted database change
//! events, so a coordinator had to be told out-of-band (a state key written
//! alongside the row) or poll.
//!
//! This is deliberately NOT change data capture. It reports mutations THIS
//! worker performed, on commit, by classifying the SQL it was given. A write
//! applied by psql, another worker, or a database-side trigger is invisible
//! here. That covers the case it exists for — agents whose only writer is this
//! worker — and nothing more, which is why it needs no logical replication, no
//! driver-specific setup, and works identically on SQLite, Postgres and MySQL.

pub mod bus;
pub mod handler;
pub mod sql;

pub use bus::{RowChangeBus, RowChangedConfig, RowChangedEvent, ROW_CHANGED_TYPE};
pub use handler::RowChangedHandler;
