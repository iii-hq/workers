//! `stories` — component stories for any React project, rendered in the iii
//! console.
//!
//! - [`config`] — the `stories` configuration entry (workspaces, data path,
//!   console URL, viewport) and its repair rules.
//! - [`compiler`] — the embedded Node compiler (Vite build, module graph,
//!   CSF manifest) materialized under the data folder.
//! - [`model`] — the index of one line: components, states, inputs; the
//!   compare algorithm and the list filters.
//! - [`store`] — the data folder: lines, content-addressed files, history.
//! - [`lines`] — resolving a line spec (working tree, ref, sha, chat turn)
//!   to a source tree on disk.
//! - [`builder`] — building a line end to end and emitting `stories:changed`.
//! - [`watch`] — the working-tree watcher.
//! - [`render`] — screenshots and DOM/React trees through the browser worker.
//! - [`pixels`] / [`treediff`] — the deterministic diff halves.
//! - [`events`] — the `stories:changed` and `stories:build` trigger types.
//! - [`functions`] — the typed `stories::*` catalog.
//! - [`ui`] — the injected console page and the `/ui-files` content function.

pub mod builder;
pub mod compiler;
pub mod config;
pub mod configuration;
pub mod events;
pub mod functions;
pub mod lines;
pub mod model;
pub mod pixels;
pub mod render;
pub mod store;
pub mod treediff;
pub mod ui;
pub mod watch;
