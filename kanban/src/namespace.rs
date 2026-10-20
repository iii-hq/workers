//! Per-instance namespace. `KANBAN_NAMESPACE=todo-local` runs this worker as
//! `kanban-todo-local`: the function ids, the trigger types, the mention
//! name, the console page ids and the asset paths all carry that name in
//! place of the leading `kanban`, and the configuration entry and the data
//! folder default to it. A second board can then sit beside the default one
//! in the same project without either seeing the other.

use std::sync::OnceLock;

pub const BASE: &str = "kanban";
pub const ENV: &str = "KANBAN_NAMESPACE";

/// The `KANBAN_NAMESPACE` value, trimmed; `None` when unset or blank.
pub fn namespace() -> Option<&'static str> {
    static NS: OnceLock<Option<String>> = OnceLock::new();
    NS.get_or_init(|| {
        std::env::var(ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
    .as_deref()
}

/// `kanban`, or `kanban-<namespace>`.
pub fn worker_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| match namespace() {
        Some(ns) => format!("{BASE}-{ns}"),
        None => BASE.to_string(),
    })
}

/// An id that starts with `kanban` (`kanban::ticket::get`, `kanban:change`,
/// `kanban-ticket`, `kanban/page.js`) with the worker name in its place.
pub fn scoped(id: &str) -> String {
    match (namespace(), id.strip_prefix(BASE)) {
        (Some(_), Some(rest)) => format!("{}{rest}", worker_name()),
        _ => id.to_string(),
    }
}

/// [`scoped`] for the SDK calls that keep a `&'static str`. Leaks once per
/// id; every caller runs at boot.
pub fn scoped_static(id: &'static str) -> &'static str {
    if namespace().is_none() {
        return id;
    }
    Box::leak(scoped(id).into_boxed_str())
}
