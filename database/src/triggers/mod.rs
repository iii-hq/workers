//! `database::row-changed` — the worker announcing its own writes.
//!
//! The database is often the state machine of a run: workers write rows and
//! something else needs to know. Until now nothing emitted database change
//! events, so a coordinator had to be told out-of-band (a state key written
//! alongside the row) or poll.
//!
//! Two capture modes, chosen per database in the worker config:
//!
//! * `statements` (default): report mutations THIS worker performed, on
//!   commit, by classifying the SQL it was given. A write applied by psql,
//!   another worker, or a database-side trigger is invisible. No database
//!   setup, works identically on SQLite, Postgres and MySQL.
//! * `native`: committed writes from ANY client fire events; table-scoped
//!   bindings only. Postgres (`native.rs`): triggers + LISTEN/NOTIFY on a
//!   dedicated connection. File-backed sqlite (`sqlite_watch.rs`): triggers →
//!   changelog table → fs-watch drain. MySQL (`mysql_binlog.rs`): the binlog
//!   replication stream — nothing installed in the schema at all.
//!
//! Native delivery is at-most-once by contract: events raised while a
//! listener is down — including the instants around a hot reload that
//! restarts it, or an interactive transaction that outlives a reload of its
//! database's config and commits on the OLD database — can be lost. It is a
//! doorbell, not a ledger; subscribers needing a gapless view reconcile on
//! their own schedule.

pub mod bus;
pub mod handler;
pub mod mysql_binlog;
pub mod native;
pub mod sql;
pub mod sqlite_watch;

pub use bus::{RowChangeBus, RowChangedConfig, RowChangedEvent, ROW_CHANGED_TYPE};
pub use handler::RowChangedHandler;
pub use native::NativeListeners;
