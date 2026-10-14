//! Native AT-SPI2 event subscription (push-based, via D-Bus signals).
//!
//! This replaces the prior inert subscription stub. AT-SPI2 delivers
//! accessibility events as D-Bus signals on four interfaces:
//!
//! - `org.a11y.atspi.Event.Object` — StateChanged, ChildrenChanged,
//!   PropertyChange, TextChanged, SelectionChanged, ValueChanged,
//!   Announcement, BoundsChanged, VisibleDataChanged, etc.
//! - `org.a11y.atspi.Event.Window` — Create/Destroy/Activate/Deactivate/…
//! - `org.a11y.atspi.Event.Focus` — Focus
//! - `org.a11y.atspi.Event.Document` — LoadComplete (not currently modelled)
//!
//! Each signal body follows the AT-SPI2 standard signature `(siiva{sv})`:
//! `(detail_kind, detail1, detail2, any_data, properties)`. The source
//! element is identified by the D-Bus message header's `sender` (bus name)
//! and `path` (object path).
//!
//! Subscriptions are scoped to a single application: we resolve the app's
//! D-Bus unique name via the registry, then install match rules filtered
//! by `sender`. A dedicated background thread iterates the message stream
//! and fans received signals out to an mpsc channel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type as MessageType;
use zbus::zvariant::OwnedValue;
use zbus::MatchRule;

use xa11y_core::{
    CancelHandle, ElementData, ElementParts, Error, Event, EventKind, EventParts, EventReceiver,
    Result, Role, StateFlag, StateParts, StateSet, Subscription, Toggled,
};

use crate::atspi::{
    is_password_text, map_atspi_role, map_atspi_role_number, AccessibleRef, LinuxProvider,
};

/// AT-SPI2 event-interface names we subscribe to via match rules.
///
/// Kept as a shared constant so registration and the interface filter in
/// `signal_to_kinds` iterate the same list.
pub(crate) const EVENT_INTERFACES: &[&str] = &[
    "org.a11y.atspi.Event.Object",
    "org.a11y.atspi.Event.Window",
    "org.a11y.atspi.Event.Focus",
    "org.a11y.atspi.Event.Document",
];

// ── Entry point ─────────────────────────────────────────────────────────────

/// Subscribe to AT-SPI2 events for the application identified by `pid`.
///
/// Returns a `Subscription` whose receiver yields `Event`s for every signal
/// emitted on the four AT-SPI event interfaces (scoped by the app's D-Bus
/// sender name). Drops and re-subscribes are clean: the cancel closure
/// removes every match rule before the thread joins.
pub(crate) fn subscribe_for_pid(
    provider: &LinuxProvider,
    pid: u32,
    app_name: String,
) -> Result<Subscription> {
    let sender_buses: Vec<String> = provider
        .find_apps_by_pid(pid)?
        .into_iter()
        .map(|app| app.bus_name)
        .collect();
    if sender_buses.is_empty() {
        return Err(Error::Platform {
            code: -1,
            message: format!("No application found with PID {pid}"),
        });
    }

    // Dedicated Connection per subscription. Dropping it (after we've
    // removed every match rule) cleanly tears the subscription down and
    // forbids any stale signal from arriving at the previous owner.
    let conn = LinuxProvider::connect_a11y_bus()?;

    // Match-rule management goes through the DBusProxy — the blocking
    // Connection itself doesn't expose AddMatch/RemoveMatch.
    let dbus = DBusProxy::new(&conn).map_err(|e| Error::Platform {
        code: -1,
        message: format!("DBusProxy: {e}"),
    })?;

    let mut rules: Vec<MatchRule<'static>> =
        Vec::with_capacity(EVENT_INTERFACES.len() * sender_buses.len());
    for sender_bus in &sender_buses {
        for iface in EVENT_INTERFACES {
            // Pass owned Strings so the produced MatchRule is `'static` and
            // can be moved into the cancel closure alongside other owned state.
            let rule = MatchRule::builder()
                .msg_type(MessageType::Signal)
                .sender(sender_bus.clone())
                .map_err(|e| Error::Platform {
                    code: -1,
                    message: format!("sender match rule: {e}"),
                })?
                .interface((*iface).to_string())
                .map_err(|e| Error::Platform {
                    code: -1,
                    message: format!("interface match rule: {e}"),
                })?
                .build();
            dbus.add_match_rule(rule.clone())
                .map_err(|e| Error::Platform {
                    code: -1,
                    message: format!("add_match_rule({iface}): {e}"),
                })?;
            rules.push(rule);
        }
    }

    let (tx, rx) = std::sync::mpsc::channel::<Event>();
    let ctx = Arc::new(EventContext {
        sender_buses: sender_buses.clone(),
        app_name,
        app_pid: pid,
        tx: Mutex::new(tx),
        conn: conn.clone(),
    });

    // Activate the broadcast receiver *before* spawning the worker. If we
    // waited until inside the spawned thread to construct the iterator,
    // there would be a window between `add_match_rule` returning above and
    // the thread scheduling its first statement during which the daemon
    // could deliver matching signals to our Connection — where they would
    // fan out to zero subscribers and be dropped. Creating the iterator
    // here makes the subscription atomic from the caller's point of view:
    // any signal that arrives after every `add_match_rule` call has
    // returned is guaranteed to land in our queue.
    let iter = MessageIterator::from(conn.clone());

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    let ctx_for_thread = ctx.clone();
    let handle = thread::spawn(move || {
        for msg in iter {
            if stop_for_thread.load(Ordering::Relaxed) {
                break;
            }
            if let Ok(msg) = msg {
                ctx_for_thread.dispatch(&msg);
            }
        }
    });

    // `CancelHandle` captures the connection and rules. On drop:
    // 1. Flip the stop flag so the thread exits on its next wake-up.
    // 2. Remove every match rule so the D-Bus daemon stops routing
    //    matching signals to us.
    // 3. Issue a Ping to the daemon — its reply travels back through our
    //    MessageIterator, forcing `.next()` to return and letting the
    //    thread observe the stop flag.
    // 4. Join the worker thread.
    let cancel_conn = conn.clone();
    let cancel = CancelHandle::new(move || {
        stop.store(true, Ordering::Relaxed);
        if let Ok(dbus) = DBusProxy::new(&cancel_conn) {
            for rule in &rules {
                let _ = dbus.remove_match_rule(rule.clone());
            }
        }
        // Wake the blocked MessageIterator by forcing a round-trip: the
        // daemon's Ping reply is broadcast through our connection's
        // message stream, so `.next()` returns and the thread observes
        // the stop flag on its next iteration.
        let _ = cancel_conn.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Peer"),
            "Ping",
            &(),
        );
        let _ = handle.join();
    });

    Ok(Subscription::new(EventReceiver::new(rx), cancel))
}

// ── Event context + dispatch ────────────────────────────────────────────────

/// Shared state between the subscription thread and the cancel path.
///
/// `tx` is wrapped in `Mutex` because `std::sync::mpsc::Sender` is `!Sync`;
/// contention is trivial (one lock per emitted event).
struct EventContext {
    sender_buses: Vec<String>,
    app_name: String,
    app_pid: u32,
    tx: Mutex<std::sync::mpsc::Sender<Event>>,
    /// Used for D-Bus property/method calls to build target snapshots.
    /// Cloned from the subscription's owning Connection, so it stays alive
    /// until every `Arc<EventContext>` (including the one held by the
    /// spawned thread) is dropped.
    conn: Connection,
}

impl EventContext {
    fn emit(&self, kind: EventKind, target: Option<ElementData>) {
        let event: Event = EventParts {
            kind,
            target,
            app_name: self.app_name.clone(),
            app_pid: self.app_pid,
            timestamp: std::time::Instant::now(),
        }
        .into();
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(event);
        }
    }

    fn dispatch(&self, msg: &zbus::Message) {
        let header = msg.header();

        // Match rules already filter by sender, but we stay defensive: the
        // daemon's Ping reply (used to wake the iterator on cancel) arrives
        // here too and must be skipped.
        let Some(sender) = header.sender() else {
            return;
        };
        if !self.sender_buses.iter().any(|bus| bus == sender.as_str()) {
            return;
        }

        let Some(iface) = header.interface() else {
            return;
        };
        let Some(member) = header.member() else {
            return;
        };
        let Some(path) = header.path() else {
            return;
        };

        // AT-SPI2 event body signature: (s, i, i, v, a{sv}).
        let body = msg.body();
        let (detail, detail1, _detail2, _any, _props): (
            String,
            i32,
            i32,
            OwnedValue,
            HashMap<String, OwnedValue>,
        ) = match body.deserialize() {
            Ok(v) => v,
            Err(_) => return,
        };

        let target_ref = AccessibleRef {
            bus_name: sender.as_str().to_string(),
            path: path.to_string(),
        };
        // Snapshot the source element synchronously so consumers receive a
        // durable `ElementData` even if the element is destroyed before
        // they read the event.
        let target = build_event_snapshot(&self.conn, &target_ref, Some(self.app_pid));
        let target_role = target.as_ref().map(|t| t.role);

        let kinds = signal_to_kinds(
            iface.as_str(),
            member.as_str(),
            &detail,
            detail1,
            target_role,
        );

        for kind in kinds {
            self.emit(kind, target.clone());
        }
    }
}

// ── Signal → EventKind mapping ──────────────────────────────────────────────

/// Map an AT-SPI2 signal tuple to zero-or-more xa11y `EventKind`s.
///
/// Returns `Vec` because some AT-SPI2 signals map to multiple cross-platform
/// kinds — e.g. `Object:StateChanged(focused, true)` emits both
/// `FocusChanged` and `StateChanged{Focused, true}` (matching the Windows
/// UIA behaviour where focus changes arrive as both a dedicated focus event
/// and a property change), and `Object:ValueChanged` on a text role emits
/// `ValueChanged` + `TextChanged` (matching macOS).
pub(crate) fn signal_to_kinds(
    iface: &str,
    member: &str,
    detail: &str,
    detail1: i32,
    target_role: Option<Role>,
) -> Vec<EventKind> {
    match (iface, member) {
        // Focus interface — keyboard focus moved.
        ("org.a11y.atspi.Event.Focus", "Focus") => vec![EventKind::FocusChanged],

        // Object.StateChanged — one signal per state bit. `detail` is the
        // state name, `detail1` is 0/1.
        ("org.a11y.atspi.Event.Object", "StateChanged") => {
            let value = detail1 != 0;
            let lower = detail.to_ascii_lowercase();
            match lower.as_str() {
                "focused" => {
                    // Some toolkits (notably GTK4) skip `Focus:Focus` and
                    // only emit StateChanged(focused). Raise both so
                    // consumers can filter on either.
                    if value {
                        vec![
                            EventKind::FocusChanged,
                            EventKind::StateChanged {
                                flag: StateFlag::Focused,
                                value,
                            },
                        ]
                    } else {
                        vec![EventKind::StateChanged {
                            flag: StateFlag::Focused,
                            value,
                        }]
                    }
                }
                "checked" => vec![EventKind::StateChanged {
                    flag: StateFlag::Checked,
                    value,
                }],
                // AT-SPI exposes both `enabled` and `sensitive`; xa11y
                // collapses them onto `Enabled` so consumers don't have to
                // deal with the (historical) distinction.
                "enabled" | "sensitive" => vec![EventKind::StateChanged {
                    flag: StateFlag::Enabled,
                    value,
                }],
                "visible" | "showing" => vec![EventKind::StateChanged {
                    flag: StateFlag::Visible,
                    value,
                }],
                "expanded" => vec![EventKind::StateChanged {
                    flag: StateFlag::Expanded,
                    value,
                }],
                "selected" => vec![EventKind::StateChanged {
                    flag: StateFlag::Selected,
                    value,
                }],
                "busy" => vec![EventKind::StateChanged {
                    flag: StateFlag::Busy,
                    value,
                }],
                "editable" => vec![EventKind::StateChanged {
                    flag: StateFlag::Editable,
                    value,
                }],
                "focusable" => vec![EventKind::StateChanged {
                    flag: StateFlag::Focusable,
                    value,
                }],
                "modal" => vec![EventKind::StateChanged {
                    flag: StateFlag::Modal,
                    value,
                }],
                "required" => vec![EventKind::StateChanged {
                    flag: StateFlag::Required,
                    value,
                }],
                // The AT-SPI state for "this window was iconified/restored".
                // Not emitted by every toolkit, but wiring it here is the only
                // way a subscriber learns of a window minimization — without
                // it the public StateFlag::Minimized is dead on Linux.
                "iconified" => vec![EventKind::StateChanged {
                    flag: StateFlag::Minimized,
                    value,
                }],
                _ => vec![],
            }
        }

        // Object.PropertyChange — AccessKit's AT-SPI bridge publishes both
        // name changes and numeric value changes through this signal with
        // `detail` set to the AT-SPI property id (`accessible-name`,
        // `accessible-value`, `accessible-description`, `accessible-parent`,
        // `accessible-role`). The design doc maps:
        // - name  → NameChanged
        // - value → ValueChanged (+ TextChanged on text roles, mirroring macOS)
        // - description/parent/role → no cross-platform EventKind; drop.
        ("org.a11y.atspi.Event.Object", "PropertyChange") => {
            let d = detail.to_ascii_lowercase();
            if d == "accessible-name" || d == "name" {
                vec![EventKind::NameChanged]
            } else if d == "accessible-value" || d == "value" {
                let mut kinds = vec![EventKind::ValueChanged];
                if matches!(target_role, Some(Role::TextField | Role::TextArea)) {
                    kinds.push(EventKind::TextChanged);
                }
                kinds
            } else {
                vec![]
            }
        }

        ("org.a11y.atspi.Event.Object", "ChildrenChanged") => vec![EventKind::StructureChanged],

        // Text changes. AT-SPI2 publishes text-change events on the Object
        // event bus (both "TextChanged" in newer specs and historical
        // "TextChanged" via an application-specific signal). `detail` is
        // "insert" or "delete".
        ("org.a11y.atspi.Event.Object", "TextChanged") => vec![EventKind::TextChanged],
        ("org.a11y.atspi.Event.Object", "TextSelectionChanged") => {
            vec![EventKind::SelectionChanged]
        }
        ("org.a11y.atspi.Event.Object", "TextAttributesChanged") => vec![],

        ("org.a11y.atspi.Event.Object", "SelectionChanged") => vec![EventKind::SelectionChanged],
        ("org.a11y.atspi.Event.Object", "ActiveDescendantChanged") => {
            vec![EventKind::SelectionChanged]
        }

        // Object.ValueChanged: AccessKit's AT-SPI bridge publishes numeric
        // value changes here (the spec moved this to Value:ValueChanged
        // eventually, but both shapes appear in the wild). Treat as
        // ValueChanged, and also emit TextChanged when the role is a text
        // role so the macOS/Windows text-field pattern survives.
        ("org.a11y.atspi.Event.Object", "ValueChanged") => {
            let mut kinds = vec![EventKind::ValueChanged];
            if matches!(target_role, Some(Role::TextField | Role::TextArea)) {
                kinds.push(EventKind::TextChanged);
            }
            kinds
        }

        ("org.a11y.atspi.Event.Object", "Announcement") => vec![EventKind::Announcement],

        // BoundsChanged / VisibleDataChanged / AttributesChanged — no cross
        // platform EventKind; skip.
        (
            "org.a11y.atspi.Event.Object",
            "BoundsChanged" | "VisibleDataChanged" | "AttributesChanged" | "ModelChanged"
            | "ColumnReordered" | "RowReordered" | "ColumnInserted" | "RowInserted"
            | "ColumnDeleted" | "RowDeleted",
        ) => vec![],

        // Window interface.
        ("org.a11y.atspi.Event.Window", "Create") => vec![EventKind::WindowOpened],
        ("org.a11y.atspi.Event.Window", "Destroy") => vec![EventKind::WindowClosed],
        ("org.a11y.atspi.Event.Window", "Activate") => vec![EventKind::WindowActivated],
        ("org.a11y.atspi.Event.Window", "Deactivate") => vec![EventKind::WindowDeactivated],
        ("org.a11y.atspi.Event.Window", "Minimize") => vec![EventKind::WindowDeactivated],
        ("org.a11y.atspi.Event.Window", "Restore") => vec![EventKind::WindowActivated],
        // Move/Resize/Maximize/Raise/Lower have no cross-platform kind.
        ("org.a11y.atspi.Event.Window", _) => vec![],

        // Document load events — not modelled cross-platform.
        ("org.a11y.atspi.Event.Document", _) => vec![],

        _ => vec![],
    }
}

// ── Snapshot helpers (free functions, independent of LinuxProvider) ─────────

/// Build a best-effort `ElementData` snapshot for an event's source element.
///
/// Returns `None` only on a catastrophic D-Bus failure — role/name/value
/// fetches are individually fallible and the snapshot survives their
/// absence (the element may have been destroyed just before we queried it).
pub(crate) fn build_event_snapshot(
    conn: &Connection,
    aref: &AccessibleRef,
    pid: Option<u32>,
) -> Option<ElementData> {
    let role_name = get_role_name(conn, aref).unwrap_or_default();
    let role_num = if role_name.is_empty() {
        get_role_number(conn, aref).unwrap_or(0)
    } else {
        0
    };
    let mut role = if !role_name.is_empty() {
        let r = map_atspi_role(&role_name);
        if r == Role::Unknown {
            let n = get_role_number(conn, aref).unwrap_or(0);
            map_atspi_role_number(n)
        } else {
            r
        }
    } else {
        map_atspi_role_number(role_num)
    };

    // State bits (used both for role refinement and for StateSet).
    // A failed GetState is `None` — preserve that as `state_read_ok = false`
    // rather than collapsing it into an empty bitset: an unread state is not
    // a state read as absent (same rule `LinuxProvider::parse_states_with_bits`
    // applies via `state_bits`), and `states_from_bits` needs it to leave
    // `minimized` unknown instead of reporting `Some(false)`.
    let (bits, state_read_ok) = match get_state(conn, aref) {
        Some(state_bits) => (bits_from_u32s(&state_bits), true),
        None => (0, false),
    };

    // Refine TextArea → TextField for single-line text widgets (mirrors
    // the role-resolution logic on the main query path).
    if role == Role::TextArea {
        const MULTI_LINE: u64 = 1 << 17;
        if (bits & MULTI_LINE) == 0 {
            role = Role::TextField;
        }
    }

    let name = get_name(conn, aref).filter(|s| !s.is_empty());
    // [P4] A password field's text never enters the process.
    let value = if is_password_text(&role_name, role_num) {
        None
    } else {
        get_value(conn, aref, role)
    };

    let (numeric_value, min_value, max_value) = if matches!(
        role,
        Role::Slider | Role::ProgressBar | Role::ScrollBar | Role::SpinButton
    ) {
        get_numeric(conn, aref)
    } else {
        (None, None, None)
    };

    let states = states_from_bits(bits, role, state_read_ok);

    let raw = {
        let raw_role = if role_name.is_empty() {
            format!("role_num:{}", role_num)
        } else {
            role_name
        };
        let mut raw = HashMap::new();
        raw.insert("atspi_role".into(), serde_json::Value::String(raw_role));
        raw.insert(
            "bus_name".into(),
            serde_json::Value::String(aref.bus_name.clone()),
        );
        raw.insert(
            "object_path".into(),
            serde_json::Value::String(aref.path.clone()),
        );
        raw
    };

    Some(
        ElementParts {
            role,
            name,
            value,
            description: None,
            bounds: None,
            actions: vec![],
            states,
            numeric_value,
            min_value,
            max_value,
            stable_id: Some(aref.path.clone()),
            pid,
            raw,
            // Snapshots are read-only targets, not live handles into the main
            // provider's cache. Consumers wanting to drive actions must
            // re-resolve through the regular locator path.
            handle: 0,
        }
        .into(),
    )
}

fn make_proxy<'a>(conn: &'a Connection, bus: &str, path: &str, iface: &str) -> Option<Proxy<'a>> {
    zbus::blocking::proxy::Builder::<Proxy>::new(conn)
        .destination(bus.to_owned())
        .ok()?
        .path(path.to_owned())
        .ok()?
        .interface(iface.to_owned())
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()
}

fn get_role_name(conn: &Connection, aref: &AccessibleRef) -> Option<String> {
    let proxy = make_proxy(
        conn,
        &aref.bus_name,
        &aref.path,
        "org.a11y.atspi.Accessible",
    )?;
    let reply = proxy.call_method("GetRoleName", &()).ok()?;
    reply.body().deserialize::<String>().ok()
}

fn get_role_number(conn: &Connection, aref: &AccessibleRef) -> Option<u32> {
    let proxy = make_proxy(
        conn,
        &aref.bus_name,
        &aref.path,
        "org.a11y.atspi.Accessible",
    )?;
    let reply = proxy.call_method("GetRole", &()).ok()?;
    reply.body().deserialize::<u32>().ok()
}

fn get_name(conn: &Connection, aref: &AccessibleRef) -> Option<String> {
    let proxy = make_proxy(
        conn,
        &aref.bus_name,
        &aref.path,
        "org.a11y.atspi.Accessible",
    )?;
    proxy.get_property::<String>("Name").ok()
}

fn get_state(conn: &Connection, aref: &AccessibleRef) -> Option<Vec<u32>> {
    let proxy = make_proxy(
        conn,
        &aref.bus_name,
        &aref.path,
        "org.a11y.atspi.Accessible",
    )?;
    let reply = proxy.call_method("GetState", &()).ok()?;
    reply.body().deserialize::<Vec<u32>>().ok()
}

fn get_value(conn: &Connection, aref: &AccessibleRef, role: Role) -> Option<String> {
    if matches!(
        role,
        Role::Application
            | Role::Window
            | Role::Dialog
            | Role::Group
            | Role::MenuBar
            | Role::Toolbar
            | Role::TabGroup
            | Role::SplitGroup
            | Role::Table
            | Role::TableRow
            | Role::Separator
    ) {
        return None;
    }

    // Prefer Text.GetText for roles that carry text (text fields, labels,
    // combo boxes) — Value.CurrentValue returns 0.0 for text elements on
    // some AT-SPI adapters.
    if let Some(proxy) = make_proxy(conn, &aref.bus_name, &aref.path, "org.a11y.atspi.Text") {
        if let Ok(char_count) = proxy.get_property::<i32>("CharacterCount") {
            if char_count > 0 {
                if let Ok(reply) = proxy.call_method("GetText", &(0i32, char_count)) {
                    if let Ok(text) = reply.body().deserialize::<String>() {
                        if !text.is_empty() {
                            return Some(text);
                        }
                    }
                }
            }
        }
    }

    if let Some(proxy) = make_proxy(conn, &aref.bus_name, &aref.path, "org.a11y.atspi.Value") {
        if let Ok(v) = proxy.get_property::<f64>("CurrentValue") {
            return Some(v.to_string());
        }
    }

    None
}

fn get_numeric(conn: &Connection, aref: &AccessibleRef) -> (Option<f64>, Option<f64>, Option<f64>) {
    let Some(proxy) = make_proxy(conn, &aref.bus_name, &aref.path, "org.a11y.atspi.Value") else {
        return (None, None, None);
    };
    (
        proxy.get_property::<f64>("CurrentValue").ok(),
        proxy.get_property::<f64>("MinimumValue").ok(),
        proxy.get_property::<f64>("MaximumValue").ok(),
    )
}

/// AT-SPI2 packs its state bitfield into a `Vec<u32>` of length 2 (low and
/// high halves). Collapse into a single u64 for easier bit-testing.
fn bits_from_u32s(state_bits: &[u32]) -> u64 {
    match state_bits.len() {
        0 => 0,
        1 => state_bits[0] as u64,
        _ => (state_bits[0] as u64) | ((state_bits[1] as u64) << 32),
    }
}

/// Convert an AT-SPI2 state bitfield into a [`StateSet`].
///
/// `state_read_ok` reports whether the bits came from a successful `GetState`
/// read. When it is `false` the read produced no evidence, and the window
/// flags must stay tri-state-unknown (`None`) rather than read-as-absent —
/// the same rule `LinuxProvider::parse_states_with_bits` applies through
/// `state_bits`, kept in sync here.
fn states_from_bits(bits: u64, role: Role, state_read_ok: bool) -> StateSet {
    // Same bit positions as `LinuxProvider::parse_states` — kept in sync.
    // Each constant is `1 << ordinal` of the `AtspiStateType` enum in
    // at-spi2-core's `atspi/atspi-constants.h` (e.g. ICONIFIED=15,
    // MODAL=16, VISIBLE=30); the state set arrives packed as one bit per
    // state ordinal.
    const ACTIVE: u64 = 1 << 1;
    const BUSY: u64 = 1 << 3;
    const CHECKED: u64 = 1 << 4;
    const EDITABLE: u64 = 1 << 7;
    const ENABLED: u64 = 1 << 8;
    const EXPANDABLE: u64 = 1 << 9;
    const EXPANDED: u64 = 1 << 10;
    const FOCUSABLE: u64 = 1 << 11;
    const FOCUSED: u64 = 1 << 12;
    const ICONIFIED: u64 = 1 << 15;
    const MODAL: u64 = 1 << 16;
    const SELECTED: u64 = 1 << 23;
    const SENSITIVE: u64 = 1 << 24;
    const SHOWING: u64 = 1 << 25;
    const VISIBLE: u64 = 1 << 30;
    const INDETERMINATE: u64 = 1 << 32;
    const REQUIRED: u64 = 1 << 33;

    let enabled = (bits & ENABLED) != 0 || (bits & SENSITIVE) != 0;
    let visible = (bits & VISIBLE) != 0 || (bits & SHOWING) != 0;

    let checked = match role {
        Role::CheckBox | Role::RadioButton | Role::MenuItem | Role::Switch => {
            if (bits & INDETERMINATE) != 0 {
                Some(Toggled::Mixed)
            } else if (bits & CHECKED) != 0 {
                Some(Toggled::On)
            } else {
                Some(Toggled::Off)
            }
        }
        _ => None,
    };

    let expanded = if (bits & EXPANDABLE) != 0 {
        Some((bits & EXPANDED) != 0)
    } else {
        None
    };

    StateParts {
        enabled,
        visible,
        focused: (bits & FOCUSED) != 0,
        // AT-SPI `ACTIVE` = foreground window/frame; kept in sync with
        // `LinuxProvider::parse_states`.
        active: (bits & ACTIVE) != 0,
        checked,
        selected: (bits & SELECTED) != 0,
        expanded,
        editable: (bits & EDITABLE) != 0,
        focusable: (bits & FOCUSABLE) != 0,
        modal: (bits & MODAL) != 0,
        required: (bits & REQUIRED) != 0,
        busy: (bits & BUSY) != 0,
        minimized: if matches!(role, Role::Window | Role::Dialog) {
            if state_read_ok {
                Some((bits & ICONIFIED) != 0)
            } else {
                // Could not read the state: say so, don't misreport the
                // window as not-minimized (mirrors `parse_states_with_bits`).
                None
            }
        } else {
            None
        },
        maximized: None,
        fullscreen: None,
    }
    .into()
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_signal_emits_focus_changed() {
        let kinds = signal_to_kinds("org.a11y.atspi.Event.Focus", "Focus", "", 0, None);
        assert_eq!(kinds, vec![EventKind::FocusChanged]);
    }

    #[test]
    fn state_changed_focused_true_emits_both_kinds() {
        // GTK4 skips Focus:Focus sometimes — `focused=true` StateChanged
        // must still raise FocusChanged so consumers don't miss it.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "StateChanged",
            "focused",
            1,
            None,
        );
        assert!(kinds.contains(&EventKind::FocusChanged));
        assert!(kinds.iter().any(|k| matches!(
            k,
            EventKind::StateChanged {
                flag: StateFlag::Focused,
                value: true
            }
        )));
    }

    #[test]
    fn state_changed_focused_false_is_state_only() {
        // Losing focus shouldn't fire FocusChanged — that's reserved for
        // the element that gained focus.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "StateChanged",
            "focused",
            0,
            None,
        );
        assert_eq!(
            kinds,
            vec![EventKind::StateChanged {
                flag: StateFlag::Focused,
                value: false
            }]
        );
    }

    #[test]
    fn state_changed_checked_maps_value() {
        let on = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "StateChanged",
            "checked",
            1,
            None,
        );
        assert_eq!(
            on,
            vec![EventKind::StateChanged {
                flag: StateFlag::Checked,
                value: true
            }]
        );
        let off = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "StateChanged",
            "checked",
            0,
            None,
        );
        assert_eq!(
            off,
            vec![EventKind::StateChanged {
                flag: StateFlag::Checked,
                value: false
            }]
        );
    }

    #[test]
    fn state_changed_enabled_and_sensitive_collapse() {
        // AT-SPI2 exposes both `enabled` and `sensitive`; xa11y's cross
        // platform model collapses them onto `Enabled`.
        for name in ["enabled", "sensitive"] {
            let kinds =
                signal_to_kinds("org.a11y.atspi.Event.Object", "StateChanged", name, 1, None);
            assert_eq!(
                kinds,
                vec![EventKind::StateChanged {
                    flag: StateFlag::Enabled,
                    value: true
                }],
                "state '{name}' should map to Enabled"
            );
        }
    }

    #[test]
    fn state_changed_iconified_maps_to_minimized() {
        // Wiring Object:StateChanged(iconified) is the only thing that makes
        // the public StateFlag::Minimized observable on Linux — without it
        // the flag would only ever surface as a snapshot, never as an event.
        for value in [0, 1] {
            let kinds = signal_to_kinds(
                "org.a11y.atspi.Event.Object",
                "StateChanged",
                "iconified",
                value,
                None,
            );
            assert_eq!(
                kinds,
                vec![EventKind::StateChanged {
                    flag: StateFlag::Minimized,
                    value: value != 0,
                }],
                "iconified={value}"
            );
        }
    }

    #[test]
    fn state_changed_unknown_state_is_dropped() {
        // Unknown AT-SPI state names must not synthesize events.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "StateChanged",
            "expandable",
            1,
            None,
        );
        assert!(kinds.is_empty());
    }

    #[test]
    fn children_changed_emits_structure_changed() {
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "ChildrenChanged",
            "add",
            0,
            None,
        );
        assert_eq!(kinds, vec![EventKind::StructureChanged]);
    }

    #[test]
    fn property_change_name_emits_name_changed() {
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "PropertyChange",
            "accessible-name",
            0,
            None,
        );
        assert_eq!(kinds, vec![EventKind::NameChanged]);
    }

    #[test]
    fn property_change_description_is_dropped() {
        // Description changes don't map to any cross-platform event kind.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "PropertyChange",
            "accessible-description",
            0,
            None,
        );
        assert!(kinds.is_empty());
    }

    #[test]
    fn property_change_value_emits_value_changed() {
        // AccessKit's AT-SPI bridge emits Object:PropertyChange with
        // detail="accessible-value" for slider/range value changes.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "PropertyChange",
            "accessible-value",
            0,
            Some(Role::Slider),
        );
        assert_eq!(kinds, vec![EventKind::ValueChanged]);
    }

    #[test]
    fn property_change_value_on_text_role_also_emits_text_changed() {
        // If a text-role element's value changes via PropertyChange, the
        // provider synthesizes TextChanged too so the macOS/Windows text
        // pattern carries over to Linux.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "PropertyChange",
            "accessible-value",
            0,
            Some(Role::TextField),
        );
        assert!(kinds.contains(&EventKind::ValueChanged));
        assert!(kinds.contains(&EventKind::TextChanged));
    }

    #[test]
    fn text_changed_maps_directly() {
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "TextChanged",
            "insert",
            0,
            None,
        );
        assert_eq!(kinds, vec![EventKind::TextChanged]);
    }

    #[test]
    fn value_changed_on_text_role_also_emits_text_changed() {
        // AccessKit's AT-SPI bridge publishes text mutations via
        // Object:ValueChanged; the events module must synthesize the
        // cross-platform TextChanged signal too so consumers match.
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "ValueChanged",
            "",
            0,
            Some(Role::TextField),
        );
        assert!(kinds.contains(&EventKind::ValueChanged));
        assert!(kinds.contains(&EventKind::TextChanged));

        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "ValueChanged",
            "",
            0,
            Some(Role::TextArea),
        );
        assert!(kinds.contains(&EventKind::TextChanged));
    }

    #[test]
    fn value_changed_on_slider_is_value_only() {
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "ValueChanged",
            "",
            0,
            Some(Role::Slider),
        );
        assert_eq!(kinds, vec![EventKind::ValueChanged]);
    }

    #[test]
    fn window_signals_map_to_window_kinds() {
        let cases = [
            ("Create", EventKind::WindowOpened),
            ("Destroy", EventKind::WindowClosed),
            ("Activate", EventKind::WindowActivated),
            ("Deactivate", EventKind::WindowDeactivated),
            ("Minimize", EventKind::WindowDeactivated),
            ("Restore", EventKind::WindowActivated),
        ];
        for (member, expected) in cases {
            let kinds = signal_to_kinds("org.a11y.atspi.Event.Window", member, "", 0, None);
            assert_eq!(
                kinds,
                vec![expected.clone()],
                "window member '{member}' should map to {expected:?}",
            );
        }
    }

    #[test]
    fn selection_changed_maps_to_selection() {
        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "SelectionChanged",
            "",
            0,
            None,
        );
        assert_eq!(kinds, vec![EventKind::SelectionChanged]);

        let kinds = signal_to_kinds(
            "org.a11y.atspi.Event.Object",
            "TextSelectionChanged",
            "",
            0,
            None,
        );
        assert_eq!(kinds, vec![EventKind::SelectionChanged]);
    }

    #[test]
    fn announcement_maps_to_announcement() {
        let kinds = signal_to_kinds("org.a11y.atspi.Event.Object", "Announcement", "", 0, None);
        assert_eq!(kinds, vec![EventKind::Announcement]);
    }

    #[test]
    fn unrecognised_signals_are_dropped() {
        // Interfaces/members that don't map to any EventKind must yield an
        // empty vec so we don't flood consumers with no-op events.
        assert!(signal_to_kinds("org.a11y.atspi.Event.Window", "Move", "", 0, None).is_empty());
        assert!(
            signal_to_kinds("org.a11y.atspi.Event.Object", "BoundsChanged", "", 0, None).is_empty()
        );
        assert!(signal_to_kinds("com.example.OtherBus", "Whatever", "", 0, None).is_empty());
    }

    #[test]
    fn event_interfaces_covers_design_doc() {
        // The design doc mandates that we subscribe to exactly these four
        // AT-SPI2 event interfaces — a future refactor that drops one
        // silently should fail this test.
        assert!(EVENT_INTERFACES.contains(&"org.a11y.atspi.Event.Object"));
        assert!(EVENT_INTERFACES.contains(&"org.a11y.atspi.Event.Window"));
        assert!(EVENT_INTERFACES.contains(&"org.a11y.atspi.Event.Focus"));
        assert!(EVENT_INTERFACES.contains(&"org.a11y.atspi.Event.Document"));
    }

    #[test]
    fn bits_from_u32s_handles_short_and_long_arrays() {
        assert_eq!(bits_from_u32s(&[]), 0);
        assert_eq!(bits_from_u32s(&[0x1234]), 0x1234);
        // The u32[1] is the high 32 bits.
        assert_eq!(bits_from_u32s(&[0x11, 0x22]), 0x22_0000_0011u64);
    }

    #[test]
    fn states_from_bits_checked_requires_toggleable_role() {
        // `Checked` only makes sense on checkbox-like roles — other roles
        // should leave it as None even when the bit is set.
        const CHECKED: u64 = 1 << 4;
        let s = states_from_bits(CHECKED, Role::Button, true);
        assert!(s.checked.is_none());
        let s = states_from_bits(CHECKED, Role::CheckBox, true);
        assert_eq!(s.checked, Some(Toggled::On));
    }

    #[test]
    fn states_from_bits_collapses_enabled_sensitive() {
        const ENABLED: u64 = 1 << 8;
        const SENSITIVE: u64 = 1 << 24;
        // Either bit on its own should flip the enabled flag.
        assert!(states_from_bits(ENABLED, Role::Button, true).enabled);
        assert!(states_from_bits(SENSITIVE, Role::Button, true).enabled);
        assert!(!states_from_bits(0, Role::Button, true).enabled);
    }

    #[test]
    fn states_from_bits_leaves_minimized_unknown_when_read_failed() {
        // A failed GetState is not evidence the window is not minimized:
        // `state_read_ok = false` must leave `minimized` as None (tri-state
        // contract, mirrored with `parse_states_with_bits`), not report
        // `Some(false)` from the defaulted zero bitset.
        const ICONIFIED: u64 = 1 << 15;
        let read_ok = states_from_bits(ICONIFIED, Role::Window, true);
        assert_eq!(read_ok.minimized, Some(true));
        let read_ok = states_from_bits(0, Role::Window, true);
        assert_eq!(read_ok.minimized, Some(false));
        let failed = states_from_bits(0, Role::Window, false);
        assert_eq!(failed.minimized, None, "unread state must stay unknown");
        // Role guard is unchanged: a non-window role stays None regardless.
        let failed = states_from_bits(0, Role::Group, false);
        assert_eq!(failed.minimized, None);
    }
}
