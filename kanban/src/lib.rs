//! `kanban` — a file-backed kanban board for iii: tickets, threaded comments,
//! agent assignment, the `kanban:change` / `kanban:comment` trigger types and
//! an injectable console board.
//!
//! - [`config`] — the `kanban` configuration entry and its repair rules.
//! - [`store`] — the board file and the serialized read/mutate/write path.
//! - [`board`] — pure ticket/comment/activity operations.
//! - [`events`] — the two trigger types and their fan-out.
//! - [`functions`] — the typed `kanban::*` function catalog.
//! - [`agents`] — assignable profiles via iii-directory or the agents folder.

pub mod agents;
pub mod board;
pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod store;
pub mod ui;
