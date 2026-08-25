//! `console::workspace::*` — open, list, and close workspace screens over
//! the bus.
//!
//! The workspace layout (tabs, columns, screens) is server-persisted in the
//! `console` configuration entry and pushed to every connected browser, so a
//! caller that writes it is showing the human something next to the
//! conversation. These functions wrap that read-modify-write with the same
//! placement rules the SPA uses (`web/src/lib/workspace-tabs.ts`); the shared
//! fixture `web/src/lib/workspace-open.fixtures.json` keeps both sides honest.

use std::collections::HashSet;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::configuration::{existing_value, set_value};

pub const CHAT_SCREEN: &str = "chat";
pub const CHAT_SESSION_SCREEN_PREFIX: &str = "chat:";
pub const EXT_SCREEN_PREFIX: &str = "ext:";
pub const ROUTED_SCREENS: [&str; 2] = ["traces", "workers"];
pub const MAX_COLUMNS: usize = 64;

const CODE_INVALID_SCREEN: &str = "WORKSPACE_INVALID_SCREEN";
const CODE_UNAVAILABLE: &str = "WORKSPACE_UNAVAILABLE";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tab {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u64>,
    pub screens: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizes: Option<Vec<f64>>,
    /// Keys this worker does not model are carried through untouched, so a
    /// newer SPA field survives a write from here.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Tab {
    pub fn column_count(&self) -> usize {
        let explicit = self.columns.map(|c| c as usize);
        let count = explicit.unwrap_or(self.screens.len());
        count.clamp(1, MAX_COLUMNS)
    }

    fn normalized_screens(&self, columns: usize) -> Vec<Option<String>> {
        (0..columns)
            .map(|i| self.screens.get(i).cloned().flatten())
            .collect()
    }

    fn normalized_sizes(&self, columns: usize) -> Vec<f64> {
        if let Some(stored) = &self.sizes {
            if stored.len() == columns && stored.iter().all(|s| s.is_finite() && *s > 0.0) {
                let total: f64 = stored.iter().sum();
                return stored.iter().map(|s| s / total).collect();
            }
        }
        vec![1.0 / columns as f64; columns]
    }

    /// Stable pane identities introduced by newer Console clients. Keep the
    /// field tolerant in `rest` so malformed future/legacy values never make
    /// the whole tab unreadable, but normalize it before structural writes.
    fn normalized_pane_ids(&self, columns: usize) -> Vec<String> {
        let stored = self
            .rest
            .get("paneIds")
            .and_then(Value::as_array)
            .and_then(|values| {
                if values.len() != columns {
                    return None;
                }
                let ids: Option<Vec<String>> = values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .map(str::to_string)
                    })
                    .collect();
                ids.filter(|ids| ids.iter().collect::<HashSet<_>>().len() == ids.len())
            });
        stored.unwrap_or_else(|| {
            (0..columns)
                .map(|index| format!("{}:pane:{index}", self.id))
                .collect()
        })
    }

    pub fn has_screen(&self, screen: &str) -> bool {
        self.screen_column(screen).is_some()
    }

    fn screen_column(&self, screen: &str) -> Option<usize> {
        self.screens
            .iter()
            .position(|s| s.as_deref() == Some(screen))
    }

    fn is_default_layout(&self) -> bool {
        self.screens.first().and_then(Option::as_deref) == Some(CHAT_SCREEN)
            && self.screens.get(1).and_then(Option::as_deref) == Some("traces")
    }

    fn with_layout(
        &self,
        screens: Vec<Option<String>>,
        sizes: Option<Vec<f64>>,
        pane_ids: Option<Vec<String>>,
    ) -> Tab {
        let mut rest = self.rest.clone();
        if let Some(pane_ids) = pane_ids {
            rest.insert("paneIds".to_string(), json!(pane_ids));
        }
        Tab {
            id: self.id.clone(),
            name: self.name.clone(),
            columns: Some(screens.len() as u64),
            screens,
            sizes,
            rest,
        }
    }
}

pub fn is_valid_screen(screen: &str) -> bool {
    is_chat_screen(screen)
        || ROUTED_SCREENS.contains(&screen)
        || screen
            .strip_prefix(EXT_SCREEN_PREFIX)
            .is_some_and(|id| !id.is_empty())
}

pub fn session_id_for_chat_screen(screen: &str) -> Option<&str> {
    screen
        .strip_prefix(CHAT_SESSION_SCREEN_PREFIX)
        .filter(|session_id| !session_id.is_empty())
}

pub fn is_chat_screen(screen: &str) -> bool {
    screen == CHAT_SCREEN || session_id_for_chat_screen(screen).is_some()
}

fn migrate_screen(screen: Option<String>) -> Option<String> {
    match screen.as_deref() {
        Some("configuration") => None,
        Some("worktrees") => Some("ext:worktree".to_string()),
        Some(s @ ("memory" | "browser" | "github")) => Some(format!("{EXT_SCREEN_PREFIX}{s}")),
        _ => screen,
    }
}

fn is_valid_tab(tab: &Tab) -> bool {
    !tab.id.is_empty()
        && tab.screens.len() <= MAX_COLUMNS
        && tab.screens.iter().flatten().all(|s| is_valid_screen(s))
        && tab
            .columns
            .is_none_or(|c| (1..=MAX_COLUMNS as u64).contains(&c))
        && tab.sizes.as_ref().is_none_or(|sizes| {
            sizes.len() <= MAX_COLUMNS && sizes.iter().all(|s| s.is_finite() && *s > 0.0)
        })
}

fn parse_tab(raw: &Value) -> Option<Tab> {
    let mut tab = serde_json::from_value::<Tab>(raw.clone()).ok()?;
    tab.screens = tab.screens.into_iter().map(migrate_screen).collect();
    is_valid_tab(&tab).then_some(tab)
}

pub fn raw_tabs(value: &Value) -> Vec<Value> {
    value
        .pointer("/workspace/tabs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub fn parse_tabs(value: &Value) -> Vec<Tab> {
    raw_tabs(value).iter().filter_map(parse_tab).collect()
}

/// Write `next` back over the stored array: a tab this worker changed is
/// re-serialized, every other stored entry (including ones this worker could
/// not parse) is kept byte-for-byte, and tabs new in `next` are appended.
pub fn merge_tabs(raw: &[Value], next: &[Tab]) -> Vec<Value> {
    let mut pending: Vec<&Tab> = next.iter().collect();
    let mut out: Vec<Value> = raw
        .iter()
        .map(|entry| {
            let Some(original) = parse_tab(entry) else {
                return entry.clone();
            };
            let Some(index) = pending.iter().position(|t| t.id == original.id) else {
                return entry.clone();
            };
            let tab = pending.remove(index);
            if *tab == original {
                entry.clone()
            } else {
                json!(tab)
            }
        })
        .collect();
    out.extend(pending.into_iter().map(|tab| json!(tab)));
    out
}

pub fn parse_active_tab_id(value: &Value) -> Option<String> {
    value
        .pointer("/workspace/activeTabId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub fn default_tabs() -> Vec<Tab> {
    vec![Tab {
        id: "tab-home".to_string(),
        name: None,
        columns: Some(2),
        screens: vec![Some(CHAT_SCREEN.to_string()), Some("traces".to_string())],
        sizes: None,
        rest: Map::new(),
    }]
}

/// The SPA's `resolveActiveTab`: the pointed tab, else the chat + traces
/// tab, else the first one.
pub fn resolve_active<'a>(tabs: &'a [Tab], pointer: Option<&str>) -> Option<&'a Tab> {
    tabs.iter()
        .find(|t| Some(t.id.as_str()) == pointer)
        .or_else(|| tabs.iter().find(|t| t.is_default_layout()))
        .or_else(|| tabs.first())
}

/// Write the layout back. `moved` says the caller changed the resolved
/// pointer: only then is it stamped as a function activation so live browsers
/// follow it; housekeeping writes keep whatever provenance the last writer
/// left, exactly like the SPA's writes.
pub fn with_workspace(value: Value, tabs: &[Value], active_tab_id: &str, moved: bool) -> Value {
    with_workspace_at(value, tabs, active_tab_id, moved, now_ms())
}

pub fn with_workspace_at(
    value: Value,
    tabs: &[Value],
    active_tab_id: &str,
    moved: bool,
    now: i64,
) -> Value {
    let mut root = match value {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let mut workspace = match root.remove("workspace") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    workspace.insert("tabs".to_string(), json!(tabs));
    if moved {
        workspace.insert("activeTabId".to_string(), json!(active_tab_id));
        workspace.insert("activatedAt".to_string(), json!(now));
        workspace.insert("activatedBy".to_string(), json!("function"));
    } else if !workspace.contains_key("activeTabId") {
        workspace.insert("activeTabId".to_string(), json!(active_tab_id));
    }
    root.insert("workspace".to_string(), Value::Object(workspace));
    Value::Object(root)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// The screen was already mounted; that tab was reused.
    Existing,
    /// Placed into an empty column of the active tab.
    EmptyColumn,
    /// A new column was added to the active tab.
    NewColumn,
    /// The active tab was full; a fresh tab was created.
    NewTab,
}

#[derive(Debug, Clone)]
pub struct Opened {
    /// `None` when the layout did not change.
    pub tabs: Option<Vec<Tab>>,
    pub tab_id: String,
    pub column: usize,
    pub placement: Placement,
    pub screens: Vec<Option<String>>,
}

/// Place `screen` beside chat in `tab`: the column right after chat when it
/// is empty, else any empty column, else a new column after chat. `None`
/// when the tab is full.
fn place_beside_chat(
    tab: &Tab,
    screen: &str,
    new_pane_id: impl FnOnce() -> String,
) -> Option<(Tab, usize, Placement)> {
    let columns = tab.column_count();
    let mut screens = tab.normalized_screens(columns);
    let chat_index = screens
        .iter()
        .position(|s| s.as_deref().is_some_and(is_chat_screen));
    let adjacent_empty = chat_index
        .map(|i| i + 1)
        .filter(|&i| i < columns && screens[i].is_none());
    let empty = adjacent_empty.or_else(|| screens.iter().position(Option::is_none));
    if let Some(index) = empty {
        screens[index] = Some(screen.to_string());
        return Some((
            tab.with_layout(screens, tab.sizes.clone(), None),
            index,
            Placement::EmptyColumn,
        ));
    }
    if columns >= MAX_COLUMNS {
        return None;
    }
    let insert_at = chat_index.map(|i| i + 1).unwrap_or(columns);
    screens.insert(insert_at, Some(screen.to_string()));
    let mut pane_ids = tab.normalized_pane_ids(columns);
    pane_ids.insert(insert_at, new_pane_id());
    let new_fraction = 1.0 / (columns + 1) as f64;
    let mut sizes: Vec<f64> = tab
        .normalized_sizes(columns)
        .into_iter()
        .map(|s| s * (1.0 - new_fraction))
        .collect();
    sizes.insert(insert_at, new_fraction);
    Some((
        tab.with_layout(screens, Some(sizes), Some(pane_ids)),
        insert_at,
        Placement::NewColumn,
    ))
}

/// Stay on the active tab when it already shows the screen, else reuse the
/// tab that does; otherwise place it beside chat in the active tab; otherwise
/// open a fresh tab. Existing screens are never replaced.
pub fn open_screen(
    tabs: &[Tab],
    active_tab_id: &str,
    screen: &str,
    new_id: impl FnOnce() -> String,
    new_pane_id: impl FnOnce() -> String,
) -> Opened {
    let active = tabs
        .iter()
        .find(|t| t.id == active_tab_id)
        .or_else(|| tabs.first());
    let mounted = active
        .filter(|t| t.has_screen(screen))
        .or_else(|| tabs.iter().find(|t| t.has_screen(screen)));
    if let Some(existing) = mounted {
        return Opened {
            tabs: None,
            tab_id: existing.id.clone(),
            column: existing.screen_column(screen).unwrap_or(0),
            placement: Placement::Existing,
            screens: existing.normalized_screens(existing.column_count()),
        };
    }
    if let Some((placed, column, placement)) =
        active.and_then(|tab| place_beside_chat(tab, screen, new_pane_id))
    {
        let placed_id = placed.id.clone();
        let mut next = tabs.to_vec();
        if let Some(slot) = next.iter_mut().find(|t| t.id == placed_id) {
            *slot = placed;
        }
        let tab = next
            .iter()
            .find(|t| t.id == placed_id)
            .expect("placed tab is in next");
        return Opened {
            tab_id: tab.id.clone(),
            column,
            placement,
            screens: tab.normalized_screens(tab.column_count()),
            tabs: Some(next),
        };
    }
    let (screens, column) = if is_chat_screen(screen) {
        (vec![Some(screen.to_string())], 0)
    } else {
        (
            vec![Some(CHAT_SCREEN.to_string()), Some(screen.to_string())],
            1,
        )
    };
    let tab = Tab {
        id: new_id(),
        name: None,
        columns: Some(screens.len() as u64),
        screens,
        sizes: None,
        rest: Map::new(),
    };
    let mut next = tabs.to_vec();
    let tab_id = tab.id.clone();
    let screens = tab.screens.clone();
    next.push(tab);
    Opened {
        tabs: Some(next),
        tab_id,
        column,
        placement: Placement::NewTab,
        screens,
    }
}

/// Detach `screen` from every tab that mounts it. A multi-column tab drops
/// the column; a single-column tab keeps the (now empty) column.
pub fn close_screen(tabs: &[Tab], screen: &str) -> (Vec<Tab>, Vec<String>) {
    let mut touched = Vec::new();
    let tabs = tabs
        .iter()
        .map(|tab| {
            let Some(index) = tab.screen_column(screen) else {
                return tab.clone();
            };
            touched.push(tab.id.clone());
            let columns = tab.column_count();
            if columns <= 1 {
                return tab.with_layout(vec![None], tab.sizes.clone(), None);
            }
            let screens = tab
                .normalized_screens(columns)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, s)| s)
                .collect();
            let kept: Vec<f64> = tab
                .normalized_sizes(columns)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, s)| s)
                .collect();
            let total: f64 = kept.iter().sum();
            let pane_ids = tab
                .normalized_pane_ids(columns)
                .into_iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, id)| id)
                .collect();
            tab.with_layout(
                screens,
                Some(kept.into_iter().map(|s| s / total).collect()),
                Some(pane_ids),
            )
        })
        .collect();
    (tabs, touched)
}

fn new_tab_id() -> String {
    format!("tab-{}", uuid::Uuid::new_v4())
}

fn new_pane_id() -> String {
    format!("pane-{}", uuid::Uuid::new_v4())
}

fn remote(code: &str, message: impl Into<String>) -> Error {
    Error::Remote {
        code: code.to_string(),
        message: message.into(),
        stacktrace: None,
    }
}

fn validated_screen(raw: &str) -> Result<String, Error> {
    let screen = raw.trim();
    if is_valid_screen(screen) {
        Ok(screen.to_string())
    } else {
        Err(remote(
            CODE_INVALID_SCREEN,
            format!(
                "screen '{screen}' is not `chat`, `chat:<session-id>`, `traces`, `workers`, or `ext:<page-id>`"
            ),
        ))
    }
}

fn validated_screen_target(
    raw_screen: &str,
    raw_session_id: Option<&str>,
) -> Result<String, Error> {
    let screen = raw_screen.trim();
    let Some(raw_session_id) = raw_session_id else {
        return validated_screen(screen);
    };
    if screen != CHAT_SCREEN {
        return Err(remote(
            CODE_INVALID_SCREEN,
            "`session_id` is only allowed when `screen` is `chat`",
        ));
    }
    let session_id = raw_session_id.trim();
    if session_id.is_empty() {
        return Err(remote(
            CODE_INVALID_SCREEN,
            "`session_id` must not be empty",
        ));
    }
    Ok(format!("{CHAT_SESSION_SCREEN_PREFIX}{session_id}"))
}

struct Layout {
    value: Value,
    raw: Vec<Value>,
    tabs: Vec<Tab>,
    active_tab_id: String,
}

impl Layout {
    async fn store(self, iii: &IIIClient, tabs: &[Tab], active_tab_id: &str) -> Result<(), Error> {
        let merged = merge_tabs(&self.raw, tabs);
        let moved = active_tab_id != self.active_tab_id;
        set_value(
            iii,
            with_workspace(self.value, &merged, active_tab_id, moved),
        )
        .await
        .map_err(|e| remote(CODE_UNAVAILABLE, e))
    }
}

async fn load_layout(iii: &IIIClient) -> Result<Layout, Error> {
    let value = existing_value(iii)
        .await
        .map_err(|e| remote(CODE_UNAVAILABLE, e))?
        .unwrap_or_else(|| json!({}));
    let raw = raw_tabs(&value);
    let mut tabs: Vec<Tab> = raw.iter().filter_map(parse_tab).collect();
    if tabs.is_empty() {
        tabs = default_tabs();
    }
    let pointer = parse_active_tab_id(&value);
    let active_tab_id = resolve_active(&tabs, pointer.as_deref())
        .map(|t| t.id.clone())
        .unwrap_or_else(|| default_tabs()[0].id.clone());
    Ok(Layout {
        value,
        raw,
        tabs,
        active_tab_id,
    })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TabSummary {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub columns: usize,
    /// One entry per column; `null` is an empty column.
    pub screens: Vec<Option<String>>,
    pub active: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub tabs: Vec<TabSummary>,
    pub active_tab_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenInput {
    /// `chat`, `traces`, `workers`, or `ext:<page-id>` for a worker page.
    pub screen: String,
    /// With `screen: "chat"`, pin the panel to this conversation session.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Make the tab holding the screen the active one (default true).
    #[serde(default)]
    pub activate: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenOutput {
    pub tab_id: String,
    pub column: usize,
    pub placement: Placement,
    pub screens: Vec<Option<String>>,
    pub activated: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseInput {
    pub screen: String,
    /// With `screen: "chat"`, close only this conversation session panel.
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CloseOutput {
    /// Tabs the screen was detached from; empty when it was not mounted.
    pub tab_ids: Vec<String>,
}

pub fn register(iii: &Arc<IIIClient>) {
    let write_lock = Arc::new(tokio::sync::Mutex::new(()));
    let client = iii.clone();
    iii.register_function(
        "console::workspace::list",
        RegisterFunction::new_async(move |_: ListInput| {
            let iii = client.clone();
            async move {
                let layout = load_layout(&iii).await?;
                let tabs = layout
                    .tabs
                    .iter()
                    .map(|t| TabSummary {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        columns: t.column_count(),
                        screens: t.normalized_screens(t.column_count()),
                        active: t.id == layout.active_tab_id,
                    })
                    .collect();
                Ok::<_, Error>(ListOutput {
                    tabs,
                    active_tab_id: layout.active_tab_id,
                })
            }
        })
        .description(
            "List the console workspace the human sees: every tab with its columns and \
             screens, plus which tab is active. Screens are `chat`, `chat:<session-id>`, \
             `traces`, `workers`, or `ext:<page>` for worker pages.",
        ),
    );

    let client = iii.clone();
    let lock = write_lock.clone();
    iii.register_function(
        "console::workspace::open",
        RegisterFunction::new_async(move |input: OpenInput| {
            let iii = client.clone();
            let lock = lock.clone();
            async move {
                let screen = validated_screen_target(&input.screen, input.session_id.as_deref())?;
                let activate = input.activate.unwrap_or(true);
                let _guard = lock.lock().await;
                let layout = load_layout(&iii).await?;
                let opened = open_screen(
                    &layout.tabs,
                    &layout.active_tab_id,
                    &screen,
                    new_tab_id,
                    new_pane_id,
                );
                let active_tab_id = if activate {
                    opened.tab_id.clone()
                } else {
                    layout.active_tab_id.clone()
                };
                let pointer_moved = active_tab_id != layout.active_tab_id;
                match opened.tabs {
                    Some(tabs) => layout.store(&iii, &tabs, &active_tab_id).await?,
                    None if pointer_moved => {
                        let tabs = layout.tabs.clone();
                        layout.store(&iii, &tabs, &active_tab_id).await?
                    }
                    None => {}
                }
                Ok::<_, Error>(OpenOutput {
                    tab_id: opened.tab_id,
                    column: opened.column,
                    placement: opened.placement,
                    screens: opened.screens,
                    activated: activate,
                })
            }
        })
        .description(
            "Show a screen to the human in the console workspace, next to the conversation. \
             Reuses the tab that already shows it, else places it beside chat in the active \
             tab, else opens a new tab. Every browser on this engine updates. Use \
             `{\"screen\":\"chat\",\"session_id\":\"<id>\"}` to open a chat panel pinned \
             to one conversation. \
             Use `ext:shell` for the file explorer, `ext:browser` for browser sessions, \
             `ext:editor` for the editor, `workers` for the worker catalog.",
        ),
    );

    let client = iii.clone();
    let lock = write_lock;
    iii.register_function(
        "console::workspace::close",
        RegisterFunction::new_async(move |input: CloseInput| {
            let iii = client.clone();
            let lock = lock.clone();
            async move {
                let screen = validated_screen_target(&input.screen, input.session_id.as_deref())?;
                let _guard = lock.lock().await;
                let layout = load_layout(&iii).await?;
                let (tabs, tab_ids) = close_screen(&layout.tabs, &screen);
                if !tab_ids.is_empty() {
                    let active_tab_id = layout.active_tab_id.clone();
                    layout.store(&iii, &tabs, &active_tab_id).await?;
                }
                Ok::<_, Error>(CloseOutput { tab_ids })
            }
        })
        .description(
            "Remove a screen from the console workspace wherever it is shown. Pass `screen: \
             chat` with `session_id` to close one pinned conversation panel. Idempotent: an \
             unmounted screen returns an empty `tab_ids`.",
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = include_str!("../../web/src/lib/workspace-open.fixtures.json");

    fn fixed_id() -> String {
        "tab-new".to_string()
    }

    fn fixed_pane_id() -> String {
        "pane-new".to_string()
    }

    fn tabs_from(value: &Value) -> Vec<Tab> {
        serde_json::from_value(value.clone()).unwrap()
    }

    #[test]
    fn screen_validation() {
        for ok in [
            "chat",
            "chat:child",
            "chat:child:attempt:2",
            "traces",
            "workers",
            "ext:shell",
            "ext:browser",
        ] {
            assert!(is_valid_screen(ok), "{ok}");
        }
        for bad in ["", "chat:", "ext:", "configuration", "settings", "http://x"] {
            assert!(!is_valid_screen(bad), "{bad}");
        }
        assert_eq!(
            session_id_for_chat_screen("chat:child:attempt:2"),
            Some("child:attempt:2")
        );
        assert_eq!(session_id_for_chat_screen("chat"), None);
    }

    #[test]
    fn structured_chat_target_validation() {
        assert_eq!(
            validated_screen_target("chat", Some("child:attempt:2")).unwrap(),
            "chat:child:attempt:2"
        );
        assert_eq!(
            validated_screen_target("chat:child", None).unwrap(),
            "chat:child"
        );
        assert!(validated_screen_target("chat", Some("")).is_err());
        assert!(validated_screen_target("chat", Some("   ")).is_err());
        assert!(validated_screen_target("workers", Some("child")).is_err());
    }

    #[test]
    fn open_matches_the_spa_fixtures() {
        let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
        for case in fixtures["open"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let tabs = tabs_from(&case["tabs"]);
            let active = case["activeTabId"].as_str().unwrap();
            let opened = open_screen(
                &tabs,
                active,
                case["screen"].as_str().unwrap(),
                fixed_id,
                fixed_pane_id,
            );
            let next = opened.tabs.clone().unwrap_or_else(|| tabs.clone());
            assert_eq!(json!(next), case["expect"]["tabs"], "tabs: {name}");
            assert_eq!(
                json!(opened.tab_id),
                case["expect"]["activeTabId"],
                "active: {name}"
            );
            let tab = next.iter().find(|t| t.id == opened.tab_id).unwrap();
            assert_eq!(
                tab.screens.get(opened.column).cloned().flatten().as_deref(),
                case["screen"].as_str(),
                "column: {name}"
            );
        }
    }

    #[test]
    fn close_matches_the_spa_fixtures() {
        let fixtures: Value = serde_json::from_str(FIXTURES).unwrap();
        for case in fixtures["close"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let tabs = tabs_from(&case["tabs"]);
            let (next, touched) = close_screen(&tabs, case["screen"].as_str().unwrap());
            assert_eq!(json!(next), case["expect"]["tabs"], "tabs: {name}");
            assert_eq!(json!(touched), case["expect"]["tabIds"], "ids: {name}");
        }
    }

    #[test]
    fn existing_screen_leaves_the_layout_untouched() {
        let tabs = tabs_from(&json!([
            { "id": "a", "columns": 2, "screens": ["chat", "ext:shell"] }
        ]));
        let opened = open_screen(&tabs, "a", "ext:shell", fixed_id, fixed_pane_id);
        assert_eq!(opened.placement, Placement::Existing);
        assert!(opened.tabs.is_none());
        assert_eq!(opened.column, 1);
    }

    #[test]
    fn structural_edits_keep_pane_ids_aligned() {
        let tabs = tabs_from(&json!([
            {
                "id": "a",
                "columns": 2,
                "screens": ["chat", "traces"],
                "paneIds": ["pane-chat", "pane-traces"]
            }
        ]));
        let opened = open_screen(&tabs, "a", "ext:shell", fixed_id, fixed_pane_id);
        let opened_tabs = opened.tabs.unwrap();
        assert_eq!(
            opened_tabs[0].rest.get("paneIds"),
            Some(&json!(["pane-chat", "pane-new", "pane-traces"]))
        );

        let (closed, touched) = close_screen(&opened_tabs, "ext:shell");
        assert_eq!(touched, vec!["a"]);
        assert_eq!(
            closed[0].rest.get("paneIds"),
            Some(&json!(["pane-chat", "pane-traces"]))
        );
    }

    #[test]
    fn full_tab_opens_a_fresh_chat_plus_screen_tab() {
        let screens: Vec<Option<String>> = (0..MAX_COLUMNS)
            .map(|i| {
                Some(if i == 0 {
                    CHAT_SCREEN.to_string()
                } else {
                    format!("ext:p{i}")
                })
            })
            .collect();
        let full = Tab {
            id: "a".to_string(),
            name: None,
            columns: Some(MAX_COLUMNS as u64),
            screens,
            sizes: None,
            rest: Map::new(),
        };
        let opened = open_screen(&[full], "a", "ext:shell", fixed_id, fixed_pane_id);
        assert_eq!(opened.placement, Placement::NewTab);
        assert_eq!(opened.tab_id, "tab-new");
        let next = opened.tabs.unwrap();
        assert_eq!(next.len(), 2);
        assert_eq!(
            next[1].screens,
            vec![Some("chat".to_string()), Some("ext:shell".to_string())]
        );
    }

    #[test]
    fn full_tab_opens_a_session_specific_chat_without_a_generic_chat() {
        let screens = (0..MAX_COLUMNS)
            .map(|i| Some(format!("ext:p{i}")))
            .collect();
        let full = Tab {
            id: "a".to_string(),
            name: None,
            columns: Some(MAX_COLUMNS as u64),
            screens,
            sizes: None,
            rest: Map::new(),
        };
        let opened = open_screen(&[full], "a", "chat:child", fixed_id, fixed_pane_id);
        assert_eq!(opened.placement, Placement::NewTab);
        assert_eq!(opened.column, 0);
        let next = opened.tabs.unwrap();
        assert_eq!(next[1].columns, Some(1));
        assert_eq!(next[1].screens, vec![Some("chat:child".to_string())]);
    }

    #[test]
    fn parse_migrates_and_filters_like_the_spa() {
        let value = json!({
            "workspace": {
                "activeTabId": "t1",
                "tabs": [
                    { "id": "t1", "columns": 2, "screens": ["chat", "configuration"], "extra": 1 },
                    { "id": "t2", "screens": ["browser", "worktrees"] },
                    { "id": "", "screens": ["chat"] },
                    { "id": "t4", "screens": ["chat", "bogus"] }
                ]
            }
        });
        let tabs = parse_tabs(&value);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].screens, vec![Some("chat".to_string()), None]);
        assert_eq!(tabs[0].rest.get("extra"), Some(&json!(1)));
        assert_eq!(
            tabs[1].screens,
            vec![
                Some("ext:browser".to_string()),
                Some("ext:worktree".to_string())
            ]
        );
        assert_eq!(parse_active_tab_id(&value).as_deref(), Some("t1"));
    }

    #[test]
    fn merge_keeps_unparsed_and_untouched_entries_verbatim() {
        let raw = vec![
            json!({ "id": "a", "columns": 2, "screens": ["chat", null], "pinned": true }),
            json!({ "id": "weird", "screens": ["from-the-future"], "x": 1 }),
            json!({ "id": "c", "screens": ["chat", "traces"] }),
        ];
        let tabs: Vec<Tab> = raw.iter().filter_map(parse_tab).collect();
        assert_eq!(tabs.len(), 2);
        let opened = open_screen(&tabs, "a", "ext:shell", fixed_id, fixed_pane_id);
        let merged = merge_tabs(&raw, &opened.tabs.unwrap());
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0]["screens"], json!(["chat", "ext:shell"]));
        assert_eq!(merged[0]["pinned"], json!(true));
        assert_eq!(merged[1], raw[1]);
        assert_eq!(merged[2], raw[2]);

        let appended = merge_tabs(
            &raw,
            &open_screen(&tabs, "c", "workers", fixed_id, fixed_pane_id)
                .tabs
                .unwrap(),
        );
        assert_eq!(appended.len(), 3);
        assert_eq!(appended[2]["screens"], json!(["chat", "workers", "traces"]));
        let brand_new = merge_tabs(&[], &default_tabs());
        assert_eq!(brand_new[0]["id"], json!("tab-home"));
    }

    #[test]
    fn empty_layouts_never_panic() {
        assert!(resolve_active(&[], Some("x")).is_none());
        let opened = open_screen(&[], "x", "workers", fixed_id, fixed_pane_id);
        assert_eq!(opened.placement, Placement::NewTab);
        assert_eq!(opened.tabs.unwrap().len(), 1);
        let stale = open_screen(
            &default_tabs(),
            "missing",
            "workers",
            fixed_id,
            fixed_pane_id,
        );
        assert_eq!(stale.tab_id, "tab-home");
    }

    #[test]
    fn with_workspace_keeps_sibling_keys() {
        let value =
            json!({ "traces": { "views": [] }, "workspace": { "tabs": [], "other": true } });
        let next = with_workspace(value, &merge_tabs(&[], &default_tabs()), "tab-home", false);
        assert_eq!(next.pointer("/traces/views"), Some(&json!([])));
        assert_eq!(next.pointer("/workspace/other"), Some(&json!(true)));
        assert_eq!(
            next.pointer("/workspace/activeTabId"),
            Some(&json!("tab-home"))
        );
        assert_eq!(
            next.pointer("/workspace/tabs/0/screens"),
            Some(&json!(["chat", "traces"]))
        );
    }

    #[test]
    fn a_moved_pointer_is_stamped_as_a_function_activation() {
        let value = json!({ "workspace": { "activeTabId": "tab-a", "activatedAt": 5, "activatedBy": "browser" } });
        let next = with_workspace_at(value, &[], "tab-b", true, 99);
        assert_eq!(next["workspace"]["activeTabId"], "tab-b");
        assert_eq!(next["workspace"]["activatedAt"], 99);
        assert_eq!(next["workspace"]["activatedBy"], "function");
    }

    #[test]
    fn an_unchanged_pointer_keeps_the_previous_activation() {
        let value = json!({ "workspace": { "activeTabId": "tab-a", "activatedAt": 5, "activatedBy": "browser" } });
        let next = with_workspace_at(value, &[], "tab-a", false, 99);
        assert_eq!(next["workspace"]["activeTabId"], "tab-a");
        assert_eq!(next["workspace"]["activatedAt"], 5);
        assert_eq!(next["workspace"]["activatedBy"], "browser");
    }

    #[test]
    fn a_housekeeping_write_over_a_missing_pointer_is_not_an_activation() {
        let next = with_workspace_at(json!({}), &[], "tab-home", false, 7);
        assert_eq!(next["workspace"]["activeTabId"], "tab-home");
        assert!(next["workspace"].get("activatedBy").is_none());
        assert!(next["workspace"].get("activatedAt").is_none());
    }

    #[test]
    fn a_stale_raw_pointer_is_not_restamped_by_a_close() {
        let value = json!({ "workspace": { "activeTabId": "gone", "activatedAt": 5, "activatedBy": "browser" } });
        let next = with_workspace_at(value, &[], "tab-home", false, 99);
        assert_eq!(next["workspace"]["activeTabId"], "gone");
        assert_eq!(next["workspace"]["activatedBy"], "browser");
    }
}
