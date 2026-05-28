//! `email` — SMTP send + real-time IMAP read worker for the iii engine.
//!
//! Surface:
//!
//!   * [`config`] — YAML-backed worker config (accounts, providers, limits).
//!   * [`handlers`] — `email::send`, `email::accounts::list`, `email::list`,
//!     `email::get`, `email::search`, `email::flag`, `email::move`,
//!     `email::attachment::get`.
//!   * [`provider::smtp`] — outbound transport via `lettre`.
//!   * [`provider::imap`] — persistent connections + IDLE push. No polling:
//!     server `EXISTS` notifications drive fan-out. If a server lacks the
//!     `IDLE` capability, the account fails to register at startup with
//!     `E610`.
//!   * [`triggers`] — `email::new-mail` trigger type, event-driven dispatcher.
//!   * [`manifest`] — `--manifest` payload exposed to the iii registry.

pub mod config;
pub mod handlers;
pub mod manifest;
pub mod provider;
pub mod triggers;

pub fn worker_name() -> &'static str {
    "email"
}
