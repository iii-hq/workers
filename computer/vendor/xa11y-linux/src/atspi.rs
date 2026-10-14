//! Real AT-SPI2 backend implementation using zbus D-Bus bindings.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use rayon::prelude::*;
use xa11y_core::{
    ElementData, ElementParts, Error, Provider, Rect, Result, Role, ShellSurfaceKind, StateParts,
    StateSet, Subscription, Toggled,
};
use zbus::blocking::{Connection, Proxy};

use crate::window::WindowManager;

/// Global handle counter for mapping ElementData back to AccessibleRefs.
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// Format a normalized state attribute as the same string `xa11y_core::selector::resolve_attr`
/// would produce, so fast-path matching agrees byte-for-byte with the slow path.
fn state_attr_to_string(name: &str, s: &StateSet) -> Option<String> {
    match name {
        "enabled" => Some(s.enabled.to_string()),
        "visible" => Some(s.visible.to_string()),
        "focused" => Some(s.focused.to_string()),
        "active" => Some(s.active.to_string()),
        "focusable" => Some(s.focusable.to_string()),
        "selected" => Some(s.selected.to_string()),
        "editable" => Some(s.editable.to_string()),
        "modal" => Some(s.modal.to_string()),
        "required" => Some(s.required.to_string()),
        "busy" => Some(s.busy.to_string()),
        "expanded" => s.expanded.map(|b| b.to_string()),
        "checked" => s.checked.map(|c| {
            match c {
                Toggled::On => "on",
                Toggled::Off => "off",
                Toggled::Mixed => "mixed",
            }
            .to_string()
        }),
        _ => None,
    }
}

/// Linux accessibility provider using AT-SPI2 over D-Bus.
pub struct LinuxProvider {
    a11y_bus: Connection,
    /// Cached AT-SPI accessible refs keyed by handle ID.
    handle_cache: Mutex<HashMap<u64, AccessibleRef>>,
    /// Cached AT-SPI2 action indices keyed by element handle.
    /// Maps each action name (snake_case) to the integer index used by `DoAction(i)`.
    action_indices: Mutex<HashMap<u64, HashMap<String, i32>>>,
    /// Native X11/Wayland window-manager channel, selected independently of
    /// the AT-SPI accessibility connection.
    window_manager: WindowManager,
}

/// AT-SPI2 accessible reference: (bus_name, object_path).
#[derive(Debug, Clone)]
pub(crate) struct AccessibleRef {
    pub(crate) bus_name: String,
    pub(crate) path: String,
}

impl LinuxProvider {
    /// Create a new Linux accessibility provider.
    ///
    /// Connects to the AT-SPI2 bus. Fails if the dedicated AT-SPI bus is
    /// unavailable (no session-bus fallback).
    pub fn new() -> Result<Self> {
        let a11y_bus = Self::connect_a11y_bus()?;
        Ok(Self {
            a11y_bus,
            handle_cache: Mutex::new(HashMap::new()),
            action_indices: Mutex::new(HashMap::new()),
            window_manager: WindowManager::new(),
        })
    }

    pub(crate) fn connect_a11y_bus() -> Result<Connection> {
        // Ask the a11y bus launcher for the AT-SPI bus address, then connect
        // to it. No fallback to the session bus: a broken a11y bus is an error.
        fn a11y_bus_error(step: &str, e: impl std::fmt::Display) -> Error {
            Error::Platform {
                code: -1,
                message: format!("AT-SPI accessibility bus unavailable ({step}): {e}"),
            }
        }
        let session = Connection::session().map_err(|e| a11y_bus_error("session bus", e))?;
        let address: String = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
            .and_then(|proxy| proxy.call("GetAddress", &()))
            .map_err(|e| a11y_bus_error("org.a11y.Bus.GetAddress", e))?;
        let address = zbus::Address::try_from(address.as_str())
            .map_err(|e| a11y_bus_error("bus address", e))?;
        zbus::blocking::connection::Builder::address(address)
            .and_then(|builder| builder.build())
            .map_err(|e| a11y_bus_error("connect", e))
    }

    fn make_proxy(&self, bus_name: &str, path: &str, interface: &str) -> Result<Proxy<'_>> {
        // Use uncached proxy to avoid GetAll calls — Qt's AT-SPI adaptor
        // doesn't support GetAll on all objects, causing spurious errors.
        zbus::blocking::proxy::Builder::<Proxy>::new(&self.a11y_bus)
            .destination(bus_name.to_owned())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Failed to set proxy destination: {}", e),
            })?
            .path(path.to_owned())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Failed to set proxy path: {}", e),
            })?
            .interface(interface.to_owned())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Failed to set proxy interface: {}", e),
            })?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Failed to create proxy: {}", e),
            })
    }

    /// Check whether an accessible object implements a given interface.
    /// Queries the AT-SPI GetInterfaces method on the Accessible interface.
    fn has_interface(&self, aref: &AccessibleRef, iface: &str) -> bool {
        let proxy = match self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible") {
            Ok(p) => p,
            Err(_) => return false,
        };
        let reply = match proxy.call_method("GetInterfaces", &()) {
            Ok(r) => r,
            Err(_) => return false,
        };
        let interfaces: Vec<String> = match reply.body().deserialize() {
            Ok(v) => v,
            Err(_) => return false,
        };
        interfaces.iter().any(|i| i.contains(iface))
    }

    /// Get the numeric AT-SPI role via GetRole method.
    fn get_role_number(&self, aref: &AccessibleRef) -> Result<u32> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = proxy
            .call_method("GetRole", &())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetRole failed: {}", e),
            })?;
        reply
            .body()
            .deserialize::<u32>()
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetRole deserialize failed: {}", e),
            })
    }

    /// Get the AT-SPI role name string.
    fn get_role_name(&self, aref: &AccessibleRef) -> Result<String> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = proxy
            .call_method("GetRoleName", &())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetRoleName failed: {}", e),
            })?;
        reply
            .body()
            .deserialize::<String>()
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetRoleName deserialize failed: {}", e),
            })
    }

    /// Detect Chromium/Electron's "accessibility bridge disabled" signature.
    ///
    /// On Linux, Chromium-based apps (Electron, Chrome, VS Code, …) register
    /// with AT-SPI but expose only an `application → frame` skeleton — the
    /// frame's children list is empty (just the `/org/a11y/atspi/null`
    /// sentinel) — unless the process was launched with the
    /// `--force-renderer-accessibility` flag. Without this detection, callers
    /// see zero results and assume their selector is wrong.
    ///
    /// Call this after observing an empty filtered children list. Returns an
    /// error only when the parent is a window/frame whose AT-SPI bus reports
    /// `Application.ToolkitName == "Chromium"`; otherwise returns `Ok(())` so
    /// genuinely empty windows in other toolkits stay valid.
    ///
    /// `role_hint` lets callers that already know the role skip a `GetRole`
    /// round-trip.
    fn check_chromium_a11y_enabled(
        &self,
        parent: &AccessibleRef,
        role_hint: Option<Role>,
    ) -> Result<()> {
        let app_root = AccessibleRef {
            bus_name: parent.bus_name.clone(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        let toolkit = match self
            .make_proxy(
                &app_root.bus_name,
                &app_root.path,
                "org.a11y.atspi.Application",
            )
            .ok()
            .and_then(|proxy| proxy.get_property::<String>("ToolkitName").ok())
        {
            Some(t) => t,
            None => return Ok(()),
        };
        if !toolkit.eq_ignore_ascii_case("Chromium") {
            return Ok(());
        }
        let role = role_hint.unwrap_or_else(|| self.resolve_role(parent));
        if role != Role::Window {
            return Ok(());
        }
        let app_name = self.get_name(&app_root).unwrap_or_default();
        Err(Error::AccessibilityNotEnabled {
            app: app_name,
            instructions: "Chromium/Electron app exposes an empty accessibility tree on Linux. \
                Relaunch with `--force-renderer-accessibility` (or set the env var \
                `ACCESSIBILITY_ENABLED=1`) so the renderer accessibility bridge is initialised."
                .to_string(),
        })
    }

    /// Get the name of an accessible element.
    fn get_name(&self, aref: &AccessibleRef) -> Result<String> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        proxy
            .get_property::<String>("Name")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Get Name property failed: {}", e),
            })
    }

    /// Get the description of an accessible element.
    fn get_description(&self, aref: &AccessibleRef) -> Result<String> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        proxy
            .get_property::<String>("Description")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Get Description property failed: {}", e),
            })
    }

    /// Get children via the GetChildren method.
    /// AT-SPI registryd doesn't always implement standard D-Bus Properties,
    /// so we use GetChildren which is more reliable.
    fn get_atspi_children(&self, aref: &AccessibleRef) -> Result<Vec<AccessibleRef>> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = proxy
            .call_method("GetChildren", &())
            .map_err(|e| children_call_error(e, aref))?;
        let children: Vec<(String, zbus::zvariant::OwnedObjectPath)> =
            reply.body().deserialize().map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetChildren deserialize failed: {}", e),
            })?;
        Ok(children
            .into_iter()
            .map(|(bus_name, path)| AccessibleRef {
                bus_name,
                path: path.to_string(),
            })
            .collect())
    }

    /// Read a live subtree, distinguishing a destroyed object from an empty
    /// child list. Only `UnknownObject` is absence; broken peers still fail.
    fn live_atspi_children(&self, aref: &AccessibleRef) -> Result<Option<Vec<AccessibleRef>>> {
        match self.get_atspi_children(aref) {
            Ok(children) => Ok(Some(children)),
            Err(Error::ElementStale { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// Get the state set as raw u32 values.
    fn get_state(&self, aref: &AccessibleRef) -> Result<Vec<u32>> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = proxy
            .call_method("GetState", &())
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetState failed: {}", e),
            })?;
        reply
            .body()
            .deserialize::<Vec<u32>>()
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetState deserialize failed: {}", e),
            })
    }

    /// `GetRole` for the shell scan, separating "the peer left the bus" from
    /// a genuine failure.
    ///
    /// `Ok(None)` means the owning application exited mid-scan; see
    /// [`PEER_GONE_ERRORS`].
    fn scan_role_number(&self, aref: &AccessibleRef) -> Result<Option<u32>> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = match proxy.call_method("GetRole", &()) {
            Ok(reply) => reply,
            Err(e) if is_peer_gone(&e) => return Ok(None),
            Err(e) => {
                return Err(Error::Platform {
                    code: -1,
                    message: format!("GetRole failed: {}", e),
                })
            }
        };
        reply
            .body()
            .deserialize::<u32>()
            .map(Some)
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetRole deserialize failed: {}", e),
            })
    }

    /// `GetAttributes` for the shell scan.
    ///
    /// Three outcomes rather than two: `Ok(None)` is the peer having left the
    /// bus, `Ok(Some(None))` is an object that does not implement
    /// `GetAttributes` (an honest "not a panel"), and `Ok(Some(Some(_)))` is a
    /// reply.
    #[allow(
        clippy::option_option,
        reason = "The two Nones mean different things \
        — peer gone versus member absent — and collapsing them would erase \
        exactly the distinction this function exists to make."
    )]
    fn scan_attributes(
        &self,
        aref: &AccessibleRef,
    ) -> Result<Option<Option<HashMap<String, String>>>> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")?;
        let reply = match proxy.call_method("GetAttributes", &()) {
            Ok(reply) => reply,
            Err(e) if is_peer_gone(&e) => return Ok(None),
            Err(e) if is_absent_member(&e) => return Ok(Some(None)),
            Err(e) => {
                return Err(Error::Platform {
                    code: -1,
                    message: format!("GetAttributes failed: {}", e),
                })
            }
        };
        reply
            .body()
            .deserialize::<HashMap<String, String>>()
            .map(|a| Some(Some(a)))
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("GetAttributes deserialize failed: {}", e),
            })
    }

    /// Return true if the element reports the AT-SPI MULTI_LINE state.
    /// Used to distinguish multi-line text areas (TextArea) from single-line
    /// text inputs (TextField), since both use the AT-SPI "text" role name.
    /// Note: Qt's AT-SPI bridge does not reliably set SINGLE_LINE, so we
    /// check MULTI_LINE and default to TextField when neither is set.
    fn is_multi_line(&self, aref: &AccessibleRef) -> bool {
        let state_bits = self.get_state(aref).unwrap_or_default();
        let bits: u64 = if state_bits.len() >= 2 {
            (state_bits[0] as u64) | ((state_bits[1] as u64) << 32)
        } else if state_bits.len() == 1 {
            state_bits[0] as u64
        } else {
            0
        };
        // ATSPI_STATE_MULTI_LINE = 17 in AtspiStateType enum
        const MULTI_LINE: u64 = 1 << 17;
        (bits & MULTI_LINE) != 0
    }

    /// Resolve TextField vs TextArea using the MULTI_LINE state, regardless of
    /// the coarse role we started with. Different toolkits expose text inputs
    /// under different AT-SPI roles ("text", "textbox", "entry", "embedded",
    /// numeric 61/78/79), and WebKitGTK has changed mapping between versions
    /// (e.g. 2.50 → 2.52 swapped what role textarea reports). MULTI_LINE is
    /// the cross-toolkit invariant, so we let it decide.
    ///
    /// For any other coarse role, returns it unchanged.
    fn refine_text_role(&self, coarse: Role, aref: &AccessibleRef) -> Role {
        match coarse {
            Role::TextArea | Role::TextField => {
                if self.is_multi_line(aref) {
                    Role::TextArea
                } else {
                    Role::TextField
                }
            }
            other => other,
        }
    }

    /// Get bounds via Component interface.
    /// Checks for Component support first to avoid GTK CRITICAL warnings
    /// on objects (e.g. TreeView cell renderers) that don't implement it.
    fn get_extents(&self, aref: &AccessibleRef) -> Option<Rect> {
        if !self.has_interface(aref, "Component") {
            return None;
        }
        let proxy = self
            .make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Component")
            .ok()?;
        // GetExtents(coord_type: u32) -> (x, y, width, height)
        // coord_type 0 = screen coordinates
        let reply = proxy.call_method("GetExtents", &(0u32,)).ok()?;
        let (x, y, w, h): (i32, i32, i32, i32) = reply.body().deserialize().ok()?;
        if w <= 0 && h <= 0 {
            return None;
        }
        // AT-SPI screen extents are physical device pixels. Convert to logical
        // coordinates (the cross-platform contract, same space as the
        // screenshot/input layers) using the detected display scale. On the
        // common 1.0 case this is a no-op. See `crate::scale`.
        Some(
            Rect {
                x,
                y,
                width: w.max(0) as u32,
                height: h.max(0) as u32,
            }
            .to_logical(crate::scale::coordinate_scale()),
        )
    }

    /// Get available actions via Action interface, returning both the action list
    /// and a map of each action name to its AT-SPI2 integer index for direct `DoAction(i)`.
    ///
    /// Probes the interface directly rather than relying on the Interfaces property,
    /// which some AT-SPI adapters (e.g. AccessKit) don't expose.
    fn get_actions(&self, aref: &AccessibleRef) -> (Vec<String>, HashMap<String, i32>) {
        let mut actions = Vec::new();
        let mut indices = HashMap::new();

        // Try Action interface directly
        if let Ok(proxy) = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Action") {
            // NActions may be returned as i32 or u32 depending on AT-SPI implementation.
            let n_actions = proxy
                .get_property::<i32>("NActions")
                .or_else(|_| proxy.get_property::<u32>("NActions").map(|n| n as i32))
                .unwrap_or(0);
            for i in 0..n_actions {
                if let Ok(reply) = proxy.call_method("GetName", &(i,)) {
                    if let Ok(name) = reply.body().deserialize::<String>() {
                        if let Some(action_name) = map_atspi_action_name(&name) {
                            if !actions.contains(&action_name) {
                                indices.insert(action_name.clone(), i);
                                actions.push(action_name);
                            }
                        }
                    }
                }
            }
        }

        // NOTE: We do NOT add implicit "focus" based on Component interface existence.
        // On GTK4, GrabFocus() often returns false even when Component interface exists,
        // violating design tenet 3 ("if action is listed, calling it must work").
        // Only report "focus" if it's explicitly in the AT-SPI Action interface with a
        // proper index. This ensures focus() will work when reported.
        // Fixes GitHub issue #98.

        (actions, indices)
    }

    /// Return true when the accessible's application identifies itself as GTK
    /// via `org.a11y.atspi.Application.ToolkitName`.
    ///
    /// Used to scope the press-fallback heuristic for the
    /// AdwMenuButton/GtkMenuButton wrapper pattern; other toolkits are
    /// unaffected. Note that WebKitGTK also reports "GTK", so the answer is
    /// not a "GtkWidget app" discriminator.
    fn is_gtk_toolkit(&self, aref: &AccessibleRef) -> bool {
        let app_root = AccessibleRef {
            bus_name: aref.bus_name.clone(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        self.make_proxy(
            &app_root.bus_name,
            &app_root.path,
            "org.a11y.atspi.Application",
        )
        .ok()
        .and_then(|proxy| proxy.get_property::<String>("ToolkitName").ok())
        .map(|t| t.eq_ignore_ascii_case("GTK"))
        .unwrap_or(false)
    }

    /// GTK press-fallback resolver.
    ///
    /// Walks the accessible's descendants (BFS, depth-capped) looking for a
    /// single activatable child that shares the outer widget's name. Returns
    /// the (ref, action_index) pair when exactly one suitable candidate
    /// exists at the shallowest matching depth. Returns `None` when the
    /// subtree contains nothing suitable or multiple equally plausible
    /// candidates — refusing to guess.
    ///
    /// Empirically fixes the `GtkMenuButton` / `AdwMenuButton` wrappers that
    /// ship in every stock GNOME app (Calculator, Text Editor, Logs, Clocks,
    /// Characters, …).
    fn find_gtk_press_fallback(
        &self,
        outer: &AccessibleRef,
        outer_name: &str,
    ) -> Option<(AccessibleRef, i32)> {
        let mut queue: VecDeque<(AccessibleRef, u32)> = VecDeque::new();
        queue.push_back((outer.clone(), 0));
        let mut visited: usize = 0;
        let mut shallowest_depth: Option<u32> = None;
        let mut hits: Vec<(AccessibleRef, i32)> = Vec::new();

        while let Some((node, depth)) = queue.pop_front() {
            // Once we've found the shallowest level with hits, don't look deeper.
            if let Some(best) = shallowest_depth {
                if depth > best {
                    continue;
                }
            }
            if visited > GTK_FALLBACK_MAX_NODES {
                break;
            }

            let role_name = if depth == 0 {
                String::new()
            } else {
                visited += 1;
                self.get_role_name(&node).unwrap_or_default().to_lowercase()
            };

            if depth > 0 && is_actionable_atspi_role(&role_name) {
                if let Some(idx) = self.gtk_fallback_pick(&node, outer_name) {
                    match shallowest_depth {
                        Some(d) if depth < d => {
                            shallowest_depth = Some(depth);
                            hits.clear();
                            hits.push((node.clone(), idx));
                        }
                        Some(d) if depth == d => hits.push((node.clone(), idx)),
                        Some(_) => {}
                        None => {
                            shallowest_depth = Some(depth);
                            hits.push((node.clone(), idx));
                        }
                    }
                }
            }

            // Do not descend into static/decorative roles. Descend through
            // containers and actionable roles alike (actionable roles may
            // themselves wrap an inner actionable — e.g. an AdwSplitButton's
            // primary button inside a toggle-button shell).
            let stop_descending = depth >= GTK_FALLBACK_MAX_DEPTH
                || (depth > 0 && is_never_descend_atspi_role(&role_name));
            if stop_descending {
                continue;
            }
            if let Ok(children) = self.get_atspi_children(&node) {
                for c in children {
                    queue.push_back((c, depth + 1));
                }
            }
        }

        if hits.len() == 1 {
            Some(hits.into_iter().next().unwrap())
        } else {
            None
        }
    }

    /// Per-node filter for `find_gtk_press_fallback`. Returns the AT-SPI
    /// action index to invoke when `aref` is a valid fallback candidate.
    fn gtk_fallback_pick(&self, aref: &AccessibleRef, outer_name: &str) -> Option<i32> {
        let (_, index_map) = self.get_actions(aref);
        let idx = *index_map.get("press")?;
        if !self.is_showing_visible_sensitive(aref) {
            return None;
        }
        // If the outer has a name, require the candidate to share it. Rules
        // out unrelated suffix widgets (e.g. a switch inside an AdwActionRow
        // that happens to expose `click`).
        if !outer_name.is_empty() {
            let inner_name = self.get_name(aref).unwrap_or_default();
            if !inner_name.is_empty() && inner_name != outer_name {
                return None;
            }
        }
        Some(idx)
    }

    /// True when the accessible's state has SHOWING, VISIBLE, and SENSITIVE
    /// set. ENABLED is deliberately excluded: in GTK4 it reflects "has an
    /// enabled GAction bound", which is false for the exact widgets we
    /// rescue.
    fn is_showing_visible_sensitive(&self, aref: &AccessibleRef) -> bool {
        let raw = self.get_state(aref).unwrap_or_default();
        let bits: u64 = if raw.len() >= 2 {
            (raw[0] as u64) | ((raw[1] as u64) << 32)
        } else if raw.len() == 1 {
            raw[0] as u64
        } else {
            0
        };
        const SENSITIVE: u64 = 1 << 24;
        const SHOWING: u64 = 1 << 25;
        const VISIBLE: u64 = 1 << 30;
        (bits & (SHOWING | VISIBLE | SENSITIVE)) == (SHOWING | VISIBLE | SENSITIVE)
    }

    /// Get value via Value or Text interface.
    /// Probes interfaces directly rather than relying on the Interfaces property.
    fn get_value(&self, aref: &AccessibleRef) -> Option<String> {
        // Try Text interface first for text content (text fields, labels, combo boxes).
        // This must come before Value because some AT-SPI adapters (e.g. AccessKit)
        // may expose both interfaces, and Value.CurrentValue returns 0.0 for text elements.
        let text_value = self.get_text_content(aref);
        if text_value.is_some() {
            return text_value;
        }
        // Try Value interface (sliders, progress bars, scroll bars, spinners)
        if let Ok(proxy) = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Value") {
            if let Ok(val) = proxy.get_property::<f64>("CurrentValue") {
                return Some(val.to_string());
            }
        }
        None
    }

    /// Read text content via the AT-SPI Text interface.
    fn get_text_content(&self, aref: &AccessibleRef) -> Option<String> {
        let proxy = self
            .make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Text")
            .ok()?;
        let char_count: i32 = proxy.get_property("CharacterCount").ok()?;
        if char_count > 0 {
            let reply = proxy.call_method("GetText", &(0i32, char_count)).ok()?;
            let text: String = reply.body().deserialize().ok()?;
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }

    /// Cache an AccessibleRef and return a new handle ID.
    fn cache_element(&self, aref: AccessibleRef) -> u64 {
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        self.handle_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle, aref);
        handle
    }

    /// Look up a cached AccessibleRef by handle.
    fn get_cached(&self, handle: u64) -> Result<AccessibleRef> {
        self.handle_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&handle)
            .cloned()
            .ok_or(Error::ElementStale {
                selector: format!("handle:{}", handle),
            })
    }

    /// Build an ElementData from an AccessibleRef, caching the ref for later lookup.
    ///
    /// After resolving the role (1-3 sequential D-Bus calls), all remaining
    /// property fetches are independent and run in parallel via rayon::join.
    fn build_element_data(&self, aref: &AccessibleRef, pid: Option<u32>) -> ElementData {
        let role_name = self.get_role_name(aref).unwrap_or_default();
        let role_num = self.get_role_number(aref).unwrap_or(0);
        // [P4] A password field's text never enters the process.
        let secret = is_password_text(&role_name, role_num);
        let coarse_role = {
            let by_name = if !role_name.is_empty() {
                map_atspi_role(&role_name)
            } else {
                Role::Unknown
            };
            let coarse = if by_name != Role::Unknown {
                by_name
            } else {
                map_atspi_role_number(role_num)
            };
            self.refine_text_role(coarse, aref)
        };
        // GTK 3 overloads AT-SPI's `menu` role: both a popup menu container
        // and the MenuItem that opens a submenu report `menu`. The entry is
        // distinguishable by its native activation action (`click`, normalized
        // to `press`); GtkMenu itself has no Action interface. Fetch once here
        // so role refinement and the advertised action list use the same
        // evidence without a second D-Bus round trip.
        let prefetched_menu_actions = if coarse_role == Role::Menu {
            Some(self.get_actions(aref))
        } else {
            None
        };
        let role = refine_menu_role(
            coarse_role,
            prefetched_menu_actions
                .as_ref()
                .map(|(actions, _)| actions.as_slice())
                .unwrap_or(&[]),
        );

        // Fetch all independent properties in parallel.
        // Left tree: (name+value, description)
        // Right tree: ((states, bounds), (actions, numeric_values))
        let (
            ((mut name, value), description),
            (
                ((states, state_bits), bounds),
                ((actions, action_index_map), (numeric_value, min_value, max_value)),
            ),
        ) = rayon::join(
            || {
                rayon::join(
                    || {
                        let name = self.get_name(aref).ok().filter(|s| !s.is_empty());
                        let value = if role_has_value(role) && !secret {
                            self.get_value(aref)
                        } else {
                            None
                        };
                        (name, value)
                    },
                    || self.get_description(aref).ok().filter(|s| !s.is_empty()),
                )
            },
            || {
                rayon::join(
                    || {
                        rayon::join(
                            || self.parse_states_with_bits(aref, role),
                            || {
                                if role != Role::Application {
                                    self.get_extents(aref)
                                } else {
                                    None
                                }
                            },
                        )
                    },
                    || {
                        rayon::join(
                            || {
                                if let Some(actions) = prefetched_menu_actions.as_ref() {
                                    actions.clone()
                                } else if role_has_actions(role) {
                                    self.get_actions(aref)
                                } else {
                                    (vec![], HashMap::new())
                                }
                            },
                            || {
                                if matches!(
                                    role,
                                    Role::Slider
                                        | Role::ProgressBar
                                        | Role::ScrollBar
                                        | Role::SpinButton
                                ) {
                                    if let Ok(proxy) = self.make_proxy(
                                        &aref.bus_name,
                                        &aref.path,
                                        "org.a11y.atspi.Value",
                                    ) {
                                        (
                                            proxy.get_property::<f64>("CurrentValue").ok(),
                                            proxy.get_property::<f64>("MinimumValue").ok(),
                                            proxy.get_property::<f64>("MaximumValue").ok(),
                                        )
                                    } else {
                                        (None, None, None)
                                    }
                                } else {
                                    (None, None, None)
                                }
                            },
                        )
                    },
                )
            },
        );

        // For label/static text elements, AT-SPI may put content in the Text
        // interface (returned as value) rather than the Name property.
        if name.is_none() && role == Role::StaticText {
            if let Some(ref v) = value {
                name = Some(v.clone());
            }
        }

        // `activate` is deliberately not advertised from AT-SPI alone. A real
        // native window backend may add it below after proving identity and
        // checking capabilities. The AT-SPI-only path is Component.GrabFocus
        // on the top-level frame, and Windows/macOS advertise theirs unconditionally,
        // but the AT-SPI adapters disagree about implementing a *frame*
        // GrabFocus: the GTK widget adapter answers true, while Chromium
        // (Electron), WebKitGTK (Tauri — whose bridge reports itself as
        // ToolkitName "GTK", so even a toolkit gate cannot separate it from
        // the GTK widget app) and Qt answer false. No interface or toolkit
        // probe discriminates them, and a snapshot-side GrabFocus probe
        // would activate windows as a side effect. Advertising anyway would
        // promise an action the call then declines — the false-promise shape
        // the integration suites exist to catch (round-4 CI: it failed the
        // activate suites on electron, tauri and qt; only the GTK cell passed).
        // The action stays callable on every toolkit, and the call reports a
        // `false` reply or a D-Bus NotSupported error honestly as
        // ActionNotSupported (tenet 3; tenet 1 — no silent fallback — is
        // why there is no substitute mechanism to advertise instead).

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
            // Preserve unstripped originals so callers who need bidi marks can
            // recover them after the strip below.
            if let Some(ref n) = name {
                raw.insert("atspi_name".into(), serde_json::Value::String(n.clone()));
            }
            if let Some(ref v) = value {
                raw.insert("atspi_value".into(), serde_json::Value::String(v.clone()));
            }
            if let Some(ref d) = description {
                raw.insert(
                    "atspi_description".into(),
                    serde_json::Value::String(d.clone()),
                );
            }
            // DEFUNCT (bit 6) marks a dead accessible — the object was
            // destroyed provider-side but a reference still resolves. It has
            // no cross-platform StateSet field, but it's the key diagnostic
            // for stale-reference bugs (e.g. a churning WebKit tree), so
            // surface it whenever set.
            if (state_bits & (1 << 6)) != 0 {
                raw.insert("atspi_defunct".into(), serde_json::Value::Bool(true));
            }
            raw
        };

        // Strip Unicode bidi format controls. RTL apps on Linux embed LRM/RLM
        // marks into reported strings; the originals are preserved in `raw`.
        let name = xa11y_core::text::strip_bidi_opt(name);
        let value = xa11y_core::text::strip_bidi_opt(value);
        let description = xa11y_core::text::strip_bidi_opt(description);

        let handle = self.cache_element(aref.clone());
        if !action_index_map.is_empty() {
            self.action_indices
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(handle, action_index_map);
        }

        let mut data: ElementData = ElementParts {
            role,
            name,
            value,
            description,
            bounds,
            actions,
            states,
            numeric_value,
            min_value,
            max_value,
            stable_id: Some(aref.path.clone()),
            pid,
            raw,
            handle,
        }
        .into();

        if matches!(role, Role::Window | Role::Dialog) && self.is_top_level_window_ref(aref) {
            let facts = self.window_manager.facts(&data);
            if facts.bounds.is_some() {
                data.bounds = facts.bounds;
            }
            for action in &facts.actions {
                if !data.actions.iter().any(|existing| existing == action) {
                    data.actions.push((*action).to_string());
                }
            }
            if facts.minimized.is_some() {
                data.states.minimized = facts.minimized;
            }
            if facts.maximized.is_some() {
                data.states.maximized = facts.maximized;
            }
            if facts.fullscreen.is_some() {
                data.states.fullscreen = facts.fullscreen;
            }
            if let Some(backend) = facts.backend {
                data.raw.insert(
                    "window_backend".to_string(),
                    serde_json::Value::String(backend.to_string()),
                );
            }
            let capabilities = [
                "activate",
                "minimize",
                "maximize",
                "enter_fullscreen",
                "restore",
                "close",
                "move_to",
                "resize_to",
            ]
            .into_iter()
            .map(|action| {
                let status = if facts.backend.is_none() {
                    "unknown"
                } else if facts.actions.contains(&action) {
                    "supported"
                } else {
                    "unsupported"
                };
                (
                    action.to_string(),
                    serde_json::Value::String(status.to_string()),
                )
            })
            .collect();
            data.raw.insert(
                "window_capabilities".to_string(),
                serde_json::Value::Object(capabilities),
            );
        }

        data
    }

    /// Get the AT-SPI parent of an accessible ref.
    fn get_atspi_parent(&self, aref: &AccessibleRef) -> Result<Option<AccessibleRef>> {
        // Read the Parent property via the D-Bus Properties interface.
        let proxy = self.make_proxy(
            &aref.bus_name,
            &aref.path,
            "org.freedesktop.DBus.Properties",
        )?;
        let reply = proxy
            .call_method("Get", &("org.a11y.atspi.Accessible", "Parent"))
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Get Parent property failed: {}", e),
            })?;
        // The reply is a Variant containing (so) — a struct of (bus_name, object_path)
        let variant: zbus::zvariant::OwnedValue =
            reply.body().deserialize().map_err(|e| Error::Platform {
                code: -1,
                message: format!("Parent deserialize variant failed: {}", e),
            })?;
        let (bus, path): (String, zbus::zvariant::OwnedObjectPath) =
            zbus::zvariant::Value::from(variant).try_into().map_err(
                |e: zbus::zvariant::Error| Error::Platform {
                    code: -1,
                    message: format!("Parent deserialize struct failed: {}", e),
                },
            )?;
        let path_str = path.as_str();
        if path_str == "/org/a11y/atspi/null" || bus.is_empty() || path_str.is_empty() {
            return Ok(None);
        }
        // If the parent is the registry root, this is a top-level app — no parent
        if path_str == "/org/a11y/atspi/accessible/root" {
            return Ok(None);
        }
        Ok(Some(AccessibleRef {
            bus_name: bus,
            path: path_str.to_string(),
        }))
    }

    /// Fetch an accessible's AT-SPI2 state bitfield as a single `u64`.
    ///
    /// AT-SPI2 returns states as two `u32`s (low/high words); collapse them
    /// into one mask so callers can test bit positions directly.
    ///
    /// The `bool` reports whether `GetState` itself succeeded. A failed read
    /// is not "all states false": window-state optionals (`minimized`) must
    /// stay unknown rather than guess `false` (tenet 1), just as the rest of
    /// the window contract treats an unread state as `None`.
    fn state_bits(&self, aref: &AccessibleRef) -> (u64, bool) {
        match self.get_state(aref) {
            Ok(state_bits) if state_bits.len() >= 2 => (
                (state_bits[0] as u64) | ((state_bits[1] as u64) << 32),
                true,
            ),
            Ok(state_bits) if state_bits.len() == 1 => (state_bits[0] as u64, true),
            Ok(_) => (0, true),
            Err(_) => (0, false),
        }
    }

    /// Parse AT-SPI2 state bitfield into xa11y StateSet.
    fn parse_states(&self, aref: &AccessibleRef, role: Role) -> StateSet {
        self.parse_states_with_bits(aref, role).0
    }

    /// Like [`parse_states`], but also returns the raw AT-SPI state bitset so
    /// callers can surface provider-only facts (notably DEFUNCT) that have no
    /// cross-platform `StateSet` field.
    fn parse_states_with_bits(&self, aref: &AccessibleRef, role: Role) -> (StateSet, u64) {
        let (bits, state_read_ok) = self.state_bits(aref);

        // AT-SPI2 state bit positions: each is `1 << ordinal` of the
        // `AtspiStateType` enum in at-spi2-core's `atspi/atspi-constants.h`
        // (ICONIFIED=15, MODAL=16, SELECTED=23, VISIBLE=30, ...). Kept in
        // sync with `states_from_bits` in events.rs.
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

        let states = StateParts {
            enabled,
            visible,
            focused: (bits & FOCUSED) != 0,
            // AT-SPI `ACTIVE` marks the foreground window/frame (see
            // `focused_app`, which relies on the same bit).
            active: (bits & ACTIVE) != 0,
            checked,
            selected: (bits & SELECTED) != 0,
            expanded,
            editable: (bits & EDITABLE) != 0,
            focusable: (bits & FOCUSABLE) != 0,
            modal: (bits & MODAL) != 0,
            required: (bits & REQUIRED) != 0,
            busy: (bits & BUSY) != 0,
            // `ICONIFIED` is AT-SPI's minimized state, reported on window
            // frames. Maximized/fullscreen are not reported by AT-SPI at all
            // (no state bit), so they stay `None` — unknown, not guessed
            // `false` (tenet 1). macOS reads the fullscreen state from
            // AXFullScreen, but no platform can raise a
            // StateChanged{fullscreen} event: the AX API has no fullscreen
            // notification and UIA has no fullscreen state either. A failed
            // `GetState` must likewise leave `minimized` unknown — an unread
            // state is not a state that was read as absent.
            minimized: if matches!(role, Role::Window | Role::Dialog) {
                if state_read_ok {
                    Some((bits & ICONIFIED) != 0)
                } else {
                    None
                }
            } else {
                None
            },
            maximized: None,
            fullscreen: None,
        }
        .into();
        (states, bits)
    }

    /// Find an application by PID.
    ///
    /// Used by `subscribe` to resolve the target app's D-Bus sender name so
    /// signal match rules can be scoped to it.
    pub(crate) fn find_apps_by_pid(&self, pid: u32) -> Result<Vec<AccessibleRef>> {
        let mut matches = Vec::new();
        let registry = AccessibleRef {
            bus_name: "org.a11y.atspi.Registry".to_string(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        let children = self.get_atspi_children(&registry)?;

        for child in &children {
            if child.path == "/org/a11y/atspi/null" {
                continue;
            }
            // Prefer D-Bus connection PID — it's authoritative for the
            // process owning the bus connection. Application.Id is *not* a
            // process pid: AT-SPI assigns it as a registry-local index
            // (1, 2, 3, …) for some bridges (notably GTK4), so matching
            // against Application.Id alone misses real processes.
            if let Some(app_pid) = self.get_dbus_pid(&child.bus_name) {
                if app_pid == pid {
                    matches.push(child.clone());
                    continue;
                }
            }
            // Fall back to Application.Id for adapters that do set it to pid.
            if let Ok(proxy) =
                self.make_proxy(&child.bus_name, &child.path, "org.a11y.atspi.Application")
            {
                if let Ok(app_pid) = proxy.get_property::<i32>("Id") {
                    if app_pid as u32 == pid {
                        matches.push(child.clone());
                    }
                }
            }
        }

        Ok(matches)
    }

    /// Get PID via D-Bus GetConnectionUnixProcessID.
    fn get_dbus_pid(&self, bus_name: &str) -> Option<u32> {
        let proxy = self
            .make_proxy(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
            )
            .ok()?;
        let reply = proxy
            .call_method("GetConnectionUnixProcessID", &(bus_name,))
            .ok()?;
        let pid: u32 = reply.body().deserialize().ok()?;
        if pid > 0 {
            Some(pid)
        } else {
            None
        }
    }

    /// [P1] After a call on `aref` failed, ask the same object for its role.
    /// `UnknownObject` / `UnknownMethod` / `UnknownInterface` (matched by the
    /// D-Bus error *name*: GDBus translates the message text) mean the object
    /// is gone, so the caller gets `ElementStale`. Any other probe outcome,
    /// success included, returns `None` and the caller keeps its own error.
    fn stale_if_gone(&self, aref: &AccessibleRef) -> Option<Error> {
        let proxy = self
            .make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Accessible")
            .ok()?;
        match proxy.call_method("GetRole", &()) {
            Err(e) if is_absent_member(&e) => Some(Error::ElementStale {
                selector: format!("{}{}", aref.bus_name, aref.path),
            }),
            _ => None,
        }
    }

    /// [P1] Classify a failed D-Bus call on `aref`: `ElementStale` when the
    /// object is gone, else `if_absent` when the call itself answered
    /// `UnknownMethod`/`UnknownInterface`/`UnknownObject`, else `Platform`.
    fn call_failed(
        &self,
        aref: &AccessibleRef,
        call: &str,
        err: zbus::Error,
        if_absent: Option<Error>,
    ) -> Error {
        self.stale_if_gone(aref).unwrap_or_else(|| match if_absent {
            Some(absent) if is_absent_member(&err) => absent,
            _ => Error::Platform {
                code: -1,
                message: format!("{call} failed: {err}"),
            },
        })
    }

    /// Perform an AT-SPI2 action by its integer index (from discovery).
    fn do_atspi_action_by_index(&self, aref: &AccessibleRef, index: i32) -> Result<()> {
        let proxy = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Action")?;
        let reply = proxy
            .call_method("DoAction", &(index,))
            .map_err(|e| self.call_failed(aref, &format!("DoAction({index})"), e, None))?;
        // AT-SPI's DoAction returns a boolean: true means the action ran. A
        // false reply means the provider declined — reporting Ok would let
        // activate()/focus() claim success while the window was never activated.
        let performed: bool = reply.body().deserialize().map_err(|e| Error::Platform {
            code: -1,
            message: format!("DoAction({}) reply decode failed: {}", index, e),
        })?;
        if !performed {
            return Err(Error::Platform {
                code: -1,
                message: format!(
                    "DoAction({}) returned false; the accessibility provider declined the action",
                    index
                ),
            });
        }
        Ok(())
    }

    /// Look up the stored AT-SPI2 action index for the given element and action.
    fn get_action_index(&self, handle: u64, action: &str) -> Result<i32> {
        self.action_indices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&handle)
            .and_then(|map| map.get(action).copied())
            .ok_or_else(|| Error::ActionNotSupported {
                action: action.to_string(),
                role: Role::Unknown, // caller will provide better context
            })
    }

    /// Get the process PID for an AT-SPI accessible. Prefers the D-Bus
    /// connection PID over `Application.Id`: the latter is a registry-local
    /// index (1, 2, 3, …) on some bridges (e.g. GTK4), not a real pid, so
    /// reading it first produces apps whose `pid` property doesn't match the
    /// process the user launched.
    fn get_app_pid(&self, aref: &AccessibleRef) -> Option<u32> {
        if let Some(pid) = self.get_dbus_pid(&aref.bus_name) {
            return Some(pid);
        }

        // Fall back to Application.Id for adapters that set it to pid.
        // Require that a process with that id actually exists — a registry
        // index from a misbehaving bridge usually names a long-dead or absurd
        // pid. This is only a cheap sanity filter: a low index can still
        // collide with a real process (1 = init), which is why the D-Bus
        // connection pid above is authoritative whenever available.
        if let Ok(proxy) = self.make_proxy(&aref.bus_name, &aref.path, "org.a11y.atspi.Application")
        {
            if let Ok(pid) = proxy.get_property::<i32>("Id") {
                if pid > 0 && std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    return Some(pid as u32);
                }
            }
        }

        None
    }

    /// Resolve the mapped Role for an accessible ref (1-3 D-Bus calls, plus
    /// an Action probe for AT-SPI `menu`, whose meaning is ambiguous on GTK 3).
    fn resolve_role(&self, aref: &AccessibleRef) -> Role {
        let role_name = self.get_role_name(aref).unwrap_or_default();
        let by_name = if !role_name.is_empty() {
            map_atspi_role(&role_name)
        } else {
            Role::Unknown
        };
        let coarse = if by_name != Role::Unknown {
            by_name
        } else {
            // Unmapped or missing role name — fall back to numeric role.
            let role_num = self.get_role_number(aref).unwrap_or(0);
            map_atspi_role_number(role_num)
        };
        let role = self.refine_text_role(coarse, aref);
        if role == Role::Menu {
            let (actions, _) = self.get_actions(aref);
            refine_menu_role(role, &actions)
        } else {
            role
        }
    }

    /// AT-SPI role-name probe for top-level decisions: is `parent` the
    /// application node?
    ///
    /// AccessKit's AT-SPI bridge reports role names as empty strings and
    /// carries the actual role as the numeric one (see `resolve_role`); an
    /// empty name fed to `map_atspi_role` would hit the strict mapper's panic
    /// arm — the exact failure the integration suite surfaced (every element's
    /// snapshot went through the round-4 top-level probe and died). The empty
    /// name is resolved numerically, exactly like `resolve_role`, so the
    /// probe agrees with the snapshot path, and a D-Bus failure is the
    /// caller's decision to make (propagated for the runtime guard).
    fn parent_role_is_application(&self, parent: &AccessibleRef) -> Result<bool> {
        let role_name = self.get_role_name(parent)?;
        let role = if role_name.is_empty() {
            map_atspi_role_number(self.get_role_number(parent)?)
        } else {
            map_atspi_role(&role_name)
        };
        Ok(role == Role::Application)
    }

    fn ensure_top_level_window_target(&self, element: &ElementData, action: &str) -> Result<()> {
        // AT-SPI window discovery is "direct child of the application node";
        // descendants with role=dialog are document structure, not OS windows.
        //
        // A Parent of the registry root (or the null address) is the AT-SPI
        // top-level shape: some adapters — AccessKit's, per the
        // `app_node_contract` integ test — report a top-level frame's Parent
        // there instead of on the application, and `get_atspi_parent` maps
        // it to `None` (see `get_parent`). `None` is therefore a
        // top-level-window answer, not a rejection: only an explicit
        // non-application parent (an in-page ARIA dialog's document ancestor)
        // means "not a real OS window".
        let target = self.get_cached(element.handle)?;
        let Some(parent) = self.get_atspi_parent(&target)? else {
            return Ok(());
        };
        // Same resolution as the advertisement probe (`is_atspi_top_level`),
        // so a window the snapshot advertised and the runtime guard can never
        // disagree; empty role names resolve numerically (AccessKit's bridge)
        // instead of feeding the strict mapper's panic arm.
        if self.parent_role_is_application(&parent)? {
            Ok(())
        } else {
            Err(Error::ActionNotSupported {
                action: action.to_string(),
                role: element.role,
            })
        }
    }

    fn is_top_level_window_ref(&self, target: &AccessibleRef) -> bool {
        match self.get_atspi_parent(target) {
            Ok(None) => true,
            Ok(Some(parent)) => self.parent_role_is_application(&parent).unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Check if an accessible ref matches a simple selector, fetching only the
    /// attributes the selector actually requires.
    ///
    /// Filter routing (cheapest first):
    ///   * `role` / `name` / `value` / `description` — single targeted D-Bus
    ///     call against this accessible.
    ///   * Normalized state attrs (`enabled`, `checked`, `focused`, …) —
    ///     one shared `GetState` call, cached for the rest of this match.
    ///   * Anything else (custom `raw` keys, `bounds`, numeric values) — fall
    ///     through to building a full `ElementData` and delegating to
    ///     [`xa11y_core::selector::matches_simple`]. This is slower but keeps
    ///     selectors with rare attribute filters semantically equivalent to
    ///     the default tree-traversal path.
    fn matches_ref(
        &self,
        aref: &AccessibleRef,
        simple: &xa11y_core::selector::SimpleSelector,
        pid: Option<u32>,
    ) -> bool {
        // Resolve role only if the selector needs it (for either the role
        // segment or any role/checked filter — checked depends on role).
        let needs_role = simple.role.is_some()
            || simple
                .filters
                .iter()
                .any(|f| matches!(f.attr.as_str(), "role" | "checked"));
        let role = if needs_role {
            Some(self.resolve_role(aref))
        } else {
            None
        };

        if let Some(ref role_match) = simple.role {
            match role_match {
                xa11y_core::selector::RoleMatch::Normalized(expected) => {
                    if role != Some(*expected) {
                        return false;
                    }
                }
                xa11y_core::selector::RoleMatch::Platform(platform_role) => {
                    let raw_role = self.get_role_name(aref).unwrap_or_default();
                    if raw_role != *platform_role {
                        return false;
                    }
                }
            }
        }

        let mut state_set: Option<StateSet> = None;

        for filter in &simple.filters {
            let attr = filter.attr.as_str();
            let resolved: Option<Option<String>> = match attr {
                "role" => Some(role.map(|r| r.to_snake_case().to_string())),
                "name" => {
                    let name = self.get_name(aref).ok().filter(|s| !s.is_empty());
                    // Mirror build_element_data: StaticText carries its name
                    // in the Text interface's Value when Name is empty.
                    let resolved = if name.is_none() && role == Some(Role::StaticText) {
                        self.get_value(aref)
                    } else {
                        name
                    };
                    Some(resolved)
                }
                // [P4] Fail closed: an unreadable role never reads the text.
                "value" => Some(match self.get_role_number(aref) {
                    Ok(role_num) if !is_password_text("", role_num) => self.get_value(aref),
                    _ => None,
                }),
                "description" => Some(self.get_description(aref).ok().filter(|s| !s.is_empty())),
                "enabled" | "visible" | "focused" | "active" | "focusable" | "selected"
                | "editable" | "modal" | "required" | "busy" | "expanded" | "checked" => {
                    let s = state_set.get_or_insert_with(|| {
                        // `parse_states` reads `checked` based on role; pass
                        // whatever we already resolved (Unknown is a no-op for
                        // the role-gated `checked` mapping).
                        self.parse_states(aref, role.unwrap_or(Role::Unknown))
                    });
                    Some(state_attr_to_string(attr, s))
                }
                _ => None, // Routed through full ElementData below.
            };

            match resolved {
                Some(value) => {
                    if !xa11y_core::selector::match_op(&filter.op, &filter.value, value.as_deref())
                    {
                        return false;
                    }
                }
                None => {
                    // Filter targets an attribute the fast path doesn't know
                    // (e.g. `bounds`, `numeric_value`, a custom `raw` key).
                    // Build the full ElementData once and let the shared
                    // matcher handle every remaining filter — it dispatches
                    // to `ElementData` fields and the `raw` map identically
                    // to the default tree-traversal path.
                    let data = self.build_element_data(aref, pid);
                    return xa11y_core::selector::matches_simple(&data, simple);
                }
            }
        }

        true
    }

    /// ONE DFS over the subtree, evaluating each clause's first SimpleSelector
    /// against every visited node. Emits `(clause_idx, AccessibleRef)` pairs
    /// in document order; a node that matches multiple clauses is emitted
    /// once per matching clause (the merge step in `find_elements_group`
    /// collapses these by `(bus_name, path)` identity).
    ///
    /// This is the "push-down to the platform query per group" core: a
    /// selector group like `button, text_field` traverses the AT-SPI tree
    /// exactly once regardless of clause count, instead of N separate walks.
    fn collect_matching_refs_group(
        &self,
        parent: &AccessibleRef,
        clauses: &[&xa11y_core::selector::SimpleSelector],
        depth: u32,
        max_depth: u32,
        limit: Option<usize>,
        pid: Option<u32>,
    ) -> Result<Vec<(usize, AccessibleRef)>> {
        if depth > max_depth {
            return Ok(vec![]);
        }

        let Some(children) = self.live_atspi_children(parent)? else {
            return Ok(Vec::new());
        };

        // Filter invalid refs. Application-node flattening (collapsing a
        // redundant `application` accessible into its grandchildren) is only
        // valid when the parent is itself an application — i.e. we have already
        // descended past the registry. At the registry root, the children *are*
        // the applications and must not be collapsed, otherwise selectors like
        // `application` (used by `App::list` / `App::by_name`) match nothing
        // because the app accessibles get dissolved into their windows.
        let parent_is_registry = parent.bus_name == "org.a11y.atspi.Registry";
        let mut to_search: Vec<AccessibleRef> = Vec::new();
        for child in children {
            if child.path == "/org/a11y/atspi/null"
                || child.bus_name.is_empty()
                || child.path.is_empty()
            {
                continue;
            }

            if !parent_is_registry {
                let child_role = self.get_role_name(&child).unwrap_or_default();
                if child_role == "application" {
                    let grandchildren = self.get_atspi_children(&child).unwrap_or_default();
                    for gc in grandchildren {
                        if gc.path == "/org/a11y/atspi/null"
                            || gc.bus_name.is_empty()
                            || gc.path.is_empty()
                        {
                            continue;
                        }
                        let gc_role = self.get_role_name(&gc).unwrap_or_default();
                        if gc_role == "application" {
                            continue;
                        }
                        to_search.push(gc);
                    }
                    continue;
                }
            }
            to_search.push(child);
        }

        // Detect Chromium/Electron's empty-tree signature whenever we
        // descend into a parent and find no real children.
        if to_search.is_empty() {
            self.check_chromium_a11y_enabled(parent, None)?;
        }

        // Process each child subtree in parallel: check match against every
        // clause + recurse. Tagging each match with `clause_idx` lets the
        // caller route phase-2 narrowing to the right clause without
        // re-running the first-segment match.
        //
        // We deliberately swallow transient sibling errors here — a single
        // flaky D-Bus call on one child shouldn't fail the whole locator
        // query (the rest of this file is similarly tolerant via
        // `unwrap_or_default()`). But `AccessibilityNotEnabled` is *not* a
        // transient error: it's the signal that a Chromium renderer bridge
        // isn't initialised, and callers need to see it. So we propagate
        // that variant specifically and keep tolerating everything else.
        // Per-child batches: matches collected during that child's subtree
        // walk, plus the first AccessibilityNotEnabled error (if any).
        type ChildBatch = (Vec<(usize, AccessibleRef)>, Option<Error>);
        let per_child: Vec<ChildBatch> = to_search
            .par_iter()
            .map(|child| {
                let mut child_results: Vec<(usize, AccessibleRef)> = Vec::new();
                for (idx, simple) in clauses.iter().enumerate() {
                    if self.matches_ref(child, simple, pid) {
                        child_results.push((idx, child.clone()));
                    }
                }
                match self.collect_matching_refs_group(
                    child,
                    clauses,
                    depth + 1,
                    max_depth,
                    limit,
                    pid,
                ) {
                    Ok(sub) => {
                        child_results.extend(sub);
                        (child_results, None)
                    }
                    Err(e @ Error::AccessibilityNotEnabled { .. }) => (Vec::new(), Some(e)),
                    Err(_) => (child_results, None),
                }
            })
            .collect();

        // Merge results, respecting limit. The first AccessibilityNotEnabled
        // error seen wins — any child subtree raising it means the whole
        // query is untrustworthy, so surface it rather than return partial
        // data.
        //
        // Note: `limit` is the *caller's* outer limit; for a multi-clause
        // group it must not be applied per-clause (a low-priority clause's
        // hit could come before a high-priority clause's in doc order, so
        // truncating mid-walk would drop legitimate matches). Callers pass
        // `None` for multi-clause queries and apply the limit after the
        // cross-clause merge.
        let mut results = Vec::new();
        for (batch, maybe_err) in per_child {
            if let Some(err) = maybe_err {
                return Err(err);
            }
            for r in batch {
                results.push(r);
                if let Some(limit) = limit {
                    if results.len() >= limit {
                        return Ok(results);
                    }
                }
            }
        }
        Ok(results)
    }
}

impl Provider for LinuxProvider {
    fn get_children(&self, element: Option<&ElementData>) -> Result<Vec<ElementData>> {
        match element {
            None => {
                // Top-level: delegate to `list_apps()`, the canonical
                // discovery primitive.
                self.list_apps()
            }
            Some(element_data) => {
                let aref = self.get_cached(element_data.handle)?;
                // A node destroyed after its snapshot has no live descendants.
                // Keep genuine D-Bus failures visible; do not run the Chromium
                // empty-tree diagnostic on a vanished window.
                let Some(children) = self.live_atspi_children(&aref)? else {
                    return Ok(Vec::new());
                };
                let pid = element_data.pid;

                // Pre-filter invalid refs and flatten nested application nodes,
                // collecting the concrete refs to build in parallel.
                let mut to_build: Vec<AccessibleRef> = Vec::new();
                for child_ref in &children {
                    if child_ref.path == "/org/a11y/atspi/null"
                        || child_ref.bus_name.is_empty()
                        || child_ref.path.is_empty()
                    {
                        continue;
                    }
                    let child_role = self.get_role_name(child_ref).unwrap_or_default();
                    if child_role == "application" {
                        // Nested application objects can also disappear between
                        // enumeration and this read. Prune only confirmed absence.
                        let Some(grandchildren) = self.live_atspi_children(child_ref)? else {
                            continue;
                        };
                        for gc_ref in grandchildren {
                            if gc_ref.path == "/org/a11y/atspi/null"
                                || gc_ref.bus_name.is_empty()
                                || gc_ref.path.is_empty()
                            {
                                continue;
                            }
                            let gc_role = self.get_role_name(&gc_ref).unwrap_or_default();
                            if gc_role == "application" {
                                continue;
                            }
                            to_build.push(gc_ref);
                        }
                        continue;
                    }
                    to_build.push(child_ref.clone());
                }

                if to_build.is_empty() {
                    self.check_chromium_a11y_enabled(&aref, Some(element_data.role))?;
                }

                let results: Vec<ElementData> = to_build
                    .par_iter()
                    .map(|r| self.build_element_data(r, pid))
                    .collect();

                Ok(results)
            }
        }
    }

    fn find_elements_group(
        &self,
        root: &ElementData,
        group: &xa11y_core::selector::SelectorGroup,
        limit: Option<usize>,
        max_depth: Option<u32>,
    ) -> Result<Vec<ElementData>> {
        if group.clauses.is_empty() {
            return Ok(vec![]);
        }
        // Reject any clause with zero segments early — `clause.segments[0]`
        // below would otherwise panic.
        if group.clauses.iter().any(|c| c.segments.is_empty()) {
            return Ok(vec![]);
        }

        // ONE AT-SPI walk that evaluates every clause's first SimpleSelector
        // against each visited node, then per-clause phase-2 narrowing on
        // the small candidate sets that fall out. Dedup uses
        // `(bus_name, path)` — the only AT-SPI identifier that's stable
        // across the per-clause narrowings (handles are minted fresh on
        // each `build_element_data` call).
        //
        // App discovery is handled separately by `list_apps()` — `root` is
        // always present here.
        let max_depth_val = max_depth.unwrap_or(xa11y_core::MAX_TREE_DEPTH);

        let firsts: Vec<&xa11y_core::selector::SimpleSelector> = group
            .clauses
            .iter()
            .map(|c| &c.segments[0].simple)
            .collect();

        let start_ref = self.get_cached(root.handle)?;

        // ── Phase-1 limit short-circuit ───────────────────────────
        // When there's exactly one clause, propagate the user's `limit`
        // (adjusted for `:nth`) to the AT-SPI walk so e.g.
        // `app.locator("button").first()` stops at the first match. With
        // multiple clauses, phase-1 hits for clause N might come before
        // clause M's in doc order, so we need the union before truncating.
        // (Critical on Electron-scale apps where this prevents 50k-node
        // walks.)
        let phase1_limit = if group.clauses.len() == 1 {
            let clause = &group.clauses[0];
            let first = firsts[0];
            let outer = if clause.segments.len() == 1 {
                limit
            } else {
                None
            };
            match (outer, first.nth) {
                (Some(l), Some(n)) => Some(l.max(n)),
                (_, Some(n)) => Some(n),
                (l, None) => l,
            }
        } else {
            None
        };

        let pid_from_root = root.pid;
        let phase1: Vec<(usize, AccessibleRef)> = self.collect_matching_refs_group(
            &start_ref,
            &firsts,
            0,
            max_depth_val,
            phase1_limit,
            pid_from_root,
        )?;

        // Bucket phase-1 hits by clause so each clause's tail can narrow
        // independently. `walk_pos` preserves the doc-order rank from the
        // single walk for the final merge.
        let mut by_clause: Vec<Vec<(usize, AccessibleRef)>> =
            (0..group.clauses.len()).map(|_| Vec::new()).collect();
        for (walk_pos, (clause_idx, aref)) in phase1.into_iter().enumerate() {
            by_clause[clause_idx].push((walk_pos, aref));
        }

        // For each clause, build ElementData for its phase-1 hits, then
        // narrow through any trailing segments. Single-segment clauses
        // skip phase 2 entirely.
        //
        // Each narrowed result keeps a (walk_pos, ElementData) pair: the
        // walk_pos of its phase-1 ancestor is the doc-order anchor we use
        // to merge across clauses at the end.
        let mut merged: Vec<(usize, ElementData)> = Vec::new();
        for (clause_idx, hits) in by_clause.into_iter().enumerate() {
            if hits.is_empty() {
                continue;
            }
            let clause = &group.clauses[clause_idx];
            // Build ElementData for this clause's phase-1 hits in parallel.
            let phase1_data: Vec<(usize, ElementData)> = hits
                .par_iter()
                .map(|(pos, aref)| {
                    // `root` is always present now, so the PID comes from
                    // the root element. No need to re-resolve via
                    // `get_app_pid` / `get_dbus_pid`.
                    (*pos, self.build_element_data(aref, pid_from_root))
                })
                .collect();

            if clause.segments.len() == 1 {
                // Apply :nth on the first segment (per-clause, then merged).
                let mut candidates = phase1_data;
                if let Some(nth) = clause.segments[0].simple.nth {
                    if nth <= candidates.len() {
                        let kept = candidates.remove(nth - 1);
                        candidates.clear();
                        candidates.push(kept);
                    } else {
                        candidates.clear();
                    }
                }
                merged.extend(candidates);
                continue;
            }

            // Multi-segment narrowing: re-use the trait helper. Each
            // narrowed result is anchored at the phase-1 ancestor's
            // walk_pos so the final sort puts it in doc order against
            // other clauses.
            for (anchor_pos, head) in phase1_data {
                let narrowed = self.narrow_multi_segment(
                    vec![head],
                    &clause.segments[1..],
                    max_depth_val,
                    None,
                )?;
                for n in narrowed {
                    merged.push((anchor_pos, n));
                }
            }
        }

        // Doc-order merge: stable sort by walk position. Within the same
        // walk position multiple clauses can have emitted the same node —
        // dedup by AT-SPI identity (`(bus_name, path)`) keeps the first.
        merged.sort_by_key(|(pos, _)| *pos);
        let mut seen: HashSet<(String, String)> = HashSet::new();
        let mut out: Vec<ElementData> = Vec::with_capacity(merged.len());
        for (_, data) in merged {
            // Use the cached AccessibleRef for identity, not the (handle).
            // Handle is freshly minted per `build_element_data` call so it
            // would never collide here.
            if let Ok(aref) = self.get_cached(data.handle) {
                if !seen.insert((aref.bus_name.clone(), aref.path.clone())) {
                    continue;
                }
            }
            out.push(data);
        }
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(out)
    }

    fn get_parent(&self, element: &ElementData) -> Result<Option<ElementData>> {
        let aref = self.get_cached(element.handle)?;
        match self.get_atspi_parent(&aref)? {
            Some(parent_ref) => {
                let data = self.build_element_data(&parent_ref, element.pid);
                Ok(Some(data))
            }
            None => Ok(None),
        }
    }

    /// Enumerate desktop panels and docks.
    ///
    /// A Linux shell surface is a top-level AT-SPI frame whose attribute set
    /// carries `window-type:dock` — how panels self-identify, from
    /// xfce4-panel's panel and dock rows to GNOME's and KDE's shells. One
    /// surface per such frame, so a panel process that owns two rows yields
    /// two [`ShellSurfaceKind::Panel`] entries, each with the panel process's
    /// pid. Panels are commonly unnamed; the frame's `name` is then `None` and
    /// the core falls back to the kind.
    ///
    /// `Panel` is the only kind Linux vends: the desktop has no menu bar,
    /// taskbar, or dock object of its own, and StatusNotifierItem *menus* live
    /// on `com.canonical.dbusmenu`, a different subsystem from AT-SPI. Their
    /// absence from the listing is honest scope, not failure.
    ///
    /// The scan runs over [`list_apps`](Self::list_apps) rather than its own
    /// registry walk, so it inherits that enumeration's empty-name filter: a
    /// panel that registers on the bus without a name is invisible here for
    /// the same reason it is invisible to `App::list`.
    ///
    /// # Errors
    ///
    /// A top-level that was just enumerated but will not answer `GetRole` or
    /// `GetAttributes` fails the listing (tenet 1). Those two calls are how a
    /// frame is classified, and `Panel` is the only kind Linux vends — so
    /// treating a D-Bus failure as "not a panel" turns one broken process into
    /// an empty listing that reads exactly like a desktop with no panels. The
    /// error names the app and object path that would not answer.
    ///
    /// An object that answers `GetAttributes` with `UnknownMethod` is the one
    /// exception, and it is absence rather than failure — see the call site.
    ///
    /// Enumerating one app's top-levels is the one step that still skips:
    /// [`get_atspi_children`](Self::get_atspi_children) failing is how a
    /// process that has left the bus presents, and `focused_app` already
    /// treats it that way.
    fn list_shell_surfaces(&self) -> Result<Vec<(ShellSurfaceKind, ElementData)>> {
        let mut surfaces = Vec::new();
        'apps: for app in self.list_apps()? {
            let app_ref = self.get_cached(app.handle)?;
            let top_levels = match self.get_atspi_children(&app_ref) {
                Ok(t) => t,
                // Same policy as `focused_app`: a single app failing to
                // enumerate its windows shouldn't abort the whole scan — skip
                // it and keep going.
                Err(_) => continue,
            };
            for top in top_levels
                .iter()
                .filter(|t| t.path != "/org/a11y/atspi/null")
            {
                // Both answer for a child the app has *just* enumerated, so a
                // D-Bus failure here is a failure, not absence — and "not a
                // panel" is the wrong reading of it (tenet 1). Say which
                // top-level of which app would not answer (tenet 6).
                //
                // `GetRole` rather than `GetRoleName` because every AT-SPI
                // object implements the numeric one, while AccessKit's bridge
                // (egui, and the other AccessKit-backed toolkits) implements
                // no `GetRoleName` at all — propagating that reply failed the
                // whole listing for a desktop running one such app.
                let Some(role_number) = self
                    .scan_role_number(top)
                    .map_err(|e| classify_failure(&app, top, e))?
                else {
                    // The application exited between `list_apps()` and this
                    // read. Its remaining top-levels are gone too, so move to
                    // the next application rather than re-asking a bus name
                    // that no longer has an owner.
                    continue 'apps;
                };
                // The same absence/failure split on the second read: an object
                // that does not implement `GetAttributes` cannot advertise
                // `window-type`, which is an honest "not a panel" — the answer
                // is in the reply, not missing from it. (AccessKit does serve
                // this one; the bridges that skip it are the reason the split
                // is here rather than only on the role.) Every other D-Bus
                // outcome — ServiceUnknown, NoReply, a timeout, a dropped
                // connection — still fails the listing.
                let Some(attributes) = self
                    .scan_attributes(top)
                    .map_err(|e| classify_failure(&app, top, e))?
                else {
                    continue 'apps;
                };
                let Some(attributes) = attributes else {
                    continue;
                };
                if is_dock_frame(role_number, &attributes) {
                    surfaces.push((
                        ShellSurfaceKind::Panel,
                        self.build_element_data(top, app.pid),
                    ));
                }
            }
        }
        Ok(surfaces)
    }

    /// A single process can register several AT-SPI Application entries
    /// (per-process instance plus per-event-loop pieces), so `App::windows`
    /// must merge their window children by pid. The same-pid merge and the
    /// stable identities it deduplicates by live in `xa11y-core`.
    fn splits_app_across_entries(&self) -> bool {
        true
    }

    fn app_roots(&self, app: &ElementData) -> Result<Vec<ElementData>> {
        let Some(pid) = app.pid else {
            return Ok(vec![app.clone()]);
        };
        let mut roots: Vec<_> = self
            .list_apps()?
            .into_iter()
            .filter(|candidate| candidate.pid == Some(pid))
            .collect();
        if roots.is_empty() {
            roots.push(app.clone());
        }
        Ok(roots)
    }

    /// Enumerate top-level applications by listing direct children of the
    /// AT-SPI registry root — every running accessibility-enabled app
    /// registers a child accessible there. Apps with empty names are
    /// filtered out (toolkits sometimes register transient/system
    /// accessibles before the real app appears). PID resolution uses the
    /// AT-SPI `Application` interface, falling back to the D-Bus owner.
    fn list_apps(&self) -> Result<Vec<ElementData>> {
        let registry = AccessibleRef {
            bus_name: "org.a11y.atspi.Registry".to_string(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        let children = self.get_atspi_children(&registry)?;

        // Filter valid children first, then build in parallel
        let valid: Vec<(&AccessibleRef, String)> = children
            .iter()
            .filter(|c| c.path != "/org/a11y/atspi/null")
            .filter_map(|c| {
                let name = self.get_name(c).unwrap_or_default();
                if name.is_empty() {
                    None
                } else {
                    Some((c, name))
                }
            })
            .collect();

        let results: Vec<ElementData> = valid
            .par_iter()
            .map(|(child, app_name)| {
                let pid = self.get_app_pid(child);
                let mut data = self.build_element_data(child, pid);
                data.name = Some(app_name.clone());
                data
            })
            .collect();

        Ok(results)
    }

    /// Find an application by pid directly against the AT-SPI registry.
    ///
    /// Differs from filtering `list_apps()` in two ways that matter for a
    /// freshly launched process:
    ///
    /// * No empty-name filter — toolkits can register on the bus before the
    ///   app name is set, and a pid match is precise enough on its own.
    /// * The not-matched error reports how many registry entries were
    ///   examined and how many had no resolvable pid (the misreported-pid
    ///   bridge quirk `get_app_pid` documents), so a timeout on CI says why
    ///   the app was invisible instead of just "not found".
    fn app_by_pid(&self, pid: u32) -> Result<ElementData> {
        let registry = AccessibleRef {
            bus_name: "org.a11y.atspi.Registry".to_string(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        let children = self.get_atspi_children(&registry)?;

        let mut total = 0usize;
        let mut unresolved = 0usize;
        for child in children.iter().filter(|c| c.path != "/org/a11y/atspi/null") {
            total += 1;
            match self.get_app_pid(child) {
                Some(p) if p == pid => {
                    let mut data = self.build_element_data(child, Some(p));
                    // Mirror list_apps: prefer the registry-reported name.
                    let name = self.get_name(child).unwrap_or_default();
                    if !name.is_empty() {
                        data.name = Some(name);
                    }
                    return Ok(data);
                }
                Some(_) => {}
                None => unresolved += 1,
            }
        }
        Err(
            Error::selector_not_matched(format!("application[pid={pid}]")).diagnose(
                xa11y_core::Diagnosis::new().last_observed(format!(
                    "{total} AT-SPI registry entries examined, {unresolved} without a \
                     resolvable pid"
                )),
            ),
        )
    }

    /// Identify the foreground application via the active top-level window.
    ///
    /// AT-SPI has no registry-level "focused application" call, and the
    /// `Application` accessibles themselves don't carry an active/focused
    /// state across toolkits. The portable signal is the `ACTIVE` state
    /// (`AtspiStateType::ACTIVE`) set on the foreground *window* (frame): we
    /// enumerate each registered application's top-level children and return
    /// the first application that owns a window reporting `ACTIVE`.
    ///
    /// Returns [`Error::SelectorNotMatched`] when no window is active (no app
    /// has the foreground — e.g. focus is on the shell or a non-AT-SPI
    /// surface). The core reads that as "nothing focused", not a failure.
    fn focused_app(&self) -> Result<ElementData> {
        // AtspiStateType::ACTIVE = 1 → bit position 1.
        const ACTIVE: u64 = 1 << 1;

        let registry = AccessibleRef {
            bus_name: "org.a11y.atspi.Registry".to_string(),
            path: "/org/a11y/atspi/accessible/root".to_string(),
        };
        let apps = self.get_atspi_children(&registry)?;

        for app in apps.iter().filter(|c| c.path != "/org/a11y/atspi/null") {
            let windows = match self.get_atspi_children(app) {
                Ok(w) => w,
                // A single app failing to enumerate its windows shouldn't abort
                // the whole foreground probe — skip it and keep scanning.
                Err(_) => continue,
            };
            let has_active_window = windows
                .iter()
                .filter(|w| w.path != "/org/a11y/atspi/null")
                .any(|w| self.state_bits(w).0 & ACTIVE != 0);
            if has_active_window {
                let pid = self.get_app_pid(app);
                let mut data = self.build_element_data(app, pid);
                let name = self.get_name(app).unwrap_or_default();
                if !name.is_empty() {
                    data.name = Some(name);
                }
                return Ok(data);
            }
        }

        Err(Error::selector_not_matched("focused application"))
    }

    fn press(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // Fast path: the widget exposes `press` on its own Action interface.
        if let Ok(index) = self.get_action_index(element.handle, "press") {
            return self.do_atspi_action_by_index(&target, index);
        }
        // TENET-BREAK(1): substitute target. Human approval granted for this
        // break (see batch B6 / PR #125 / commit daeaf59). Justification: GTK4
        // menu-button widgets (GtkMenuButton, AdwMenuButton, AdwSplitButton)
        // expose their AT-SPI Action interface on an *inner* toggle-button
        // child, not the outer push-button accessible the app author addresses
        // by name. Without this workaround, `press()` silently does nothing on
        // every GtkMenuButton in every stock GNOME app (Calculator, Text
        // Editor, Logs, …). Alternatives considered and rejected:
        //   1. Expose the inner widget in the tree — breaks AT-SPI tree
        //      fidelity and leaks a GTK implementation detail into every
        //      consumer.
        //   2. Return ActionNotSupported — every GTK consumer would have to
        //      reimplement the same subtree search to work around it.
        // The break is narrowly scoped: gated on (a) the owning toolkit being
        // GTK and (b) the outer role being Role::Button (push-button, which is
        // what all three menu-button variants present as). Non-GtkMenuButton
        // roles and non-GTK toolkits continue to fail-fast with
        // ActionNotSupported.
        if element.role == Role::Button && self.is_gtk_toolkit(&target) {
            let outer_name = self.get_name(&target).unwrap_or_default();
            if let Some((inner, index)) = self.find_gtk_press_fallback(&target, &outer_name) {
                return self.do_atspi_action_by_index(&inner, index);
            }
        }
        Err(Error::ActionNotSupported {
            action: "press".to_string(),
            role: element.role,
        })
    }

    fn focus(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // The only mechanism is Component.GrabFocus, which returns a boolean
        // indicating success — we must check it. Fixes GitHub issue #98.
        //
        // There is deliberately no stored-action-index fallback: one cannot
        // fire. `action_indices` is populated only for roles in
        // `role_has_actions` (Window/Dialog excluded) from names
        // `map_atspi_action_name` produces, and no mapping yields "focus" (see
        // `get_actions`'s note: "focus" is only ever reported when explicitly
        // in the AT-SPI Action interface). Keeping the dead lookup here would
        // pretend to a second mechanism that never succeeds — the same
        // decision `activate` documents above (tenet 1).
        let proxy = self
            .make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Component")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Component proxy while focusing failed: {}", e),
            })?;
        let reply = grab_focus_result(
            proxy.call_method("GrabFocus", &()),
            "focusing",
            "focus",
            element.role,
        )?;
        let grabbed: bool = reply.body().deserialize().map_err(|e| Error::Platform {
            code: -1,
            message: format!("Component.GrabFocus reply while focusing failed: {}", e),
        })?;
        if grabbed {
            return Ok(());
        }
        // GrabFocus(false) is an honest "cannot focus this element" — the
        // AT-SPI adapter declined, not a platform failure.
        Err(Error::ActionNotSupported {
            action: "focus".to_string(),
            role: element.role,
        })
    }

    fn blur(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // Grab focus on parent element to blur the current one. We propagate
        // every failure — no silent fallbacks (tenet 1): callers need to see
        // when blur can't do anything useful.
        if let Some(parent_ref) = self.get_atspi_parent(&target)? {
            if parent_ref.path != "/org/a11y/atspi/null" {
                let p = self.make_proxy(
                    &parent_ref.bus_name,
                    &parent_ref.path,
                    "org.a11y.atspi.Component",
                )?;
                p.call_method("GrabFocus", &())
                    .map_err(|e| Error::Platform {
                        code: -1,
                        message: format!("Component.GrabFocus on parent failed: {}", e),
                    })?;
                return Ok(());
            }
        }
        Err(Error::ActionNotSupported {
            action: "blur".to_string(),
            role: element.role,
        })
    }

    fn toggle(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // Fast path: the widget exposes the literal AT-SPI action name "toggle"
        // (or one of its toolkit aliases — "check" / "uncheck" — both
        // canonicalised to "toggle" by map_atspi_action_name during cache
        // population).
        if let Ok(index) = self.get_action_index(element.handle, "toggle") {
            return self.do_atspi_action_by_index(&target, index);
        }
        // Fallback for AccessKit-backed apps. accesskit_unix's AT-SPI bridge
        // hard-codes the action name as "click" for every clickable node
        // regardless of role — see platforms/atspi-common/src/node.rs in
        // AccessKit. That means egui, eframe, winit-based apps, Bevy, Tauri
        // (when using AccessKit) and any other AccessKit consumer never
        // advertise "toggle" for CheckBox/RadioButton/Switch, even though
        // activating those widgets via "click" is functionally a toggle on
        // every platform. Without this fallback, xa11y.toggle() is
        // unreachable on every AccessKit-on-Linux widget.
        //
        // This is tenet-3-consistent: the semantic verb `toggle` on a
        // toggleable role means "activate this toggleable element," and on
        // Linux the canonical AccessKit implementation of that is the same
        // action node entry we'd dispatch for press(). It's the mirror image
        // of the Windows `press` dispatch (uia.rs) which fans out to Invoke,
        // Toggle, SelectionItem.Select, or ExpandCollapse depending on the
        // element's primary-activation pattern.
        //
        // Narrowly scoped: only fires for toggleable primary-activation
        // roles. Other roles still fail-fast with ActionNotSupported so a
        // misuse of toggle() on, e.g., a Button does not silently dispatch
        // press().
        if matches!(
            element.role,
            Role::CheckBox | Role::RadioButton | Role::Switch
        ) {
            if let Ok(index) = self.get_action_index(element.handle, "press") {
                return self.do_atspi_action_by_index(&target, index);
            }
        }
        Err(Error::ActionNotSupported {
            action: "toggle".to_string(),
            role: element.role,
        })
    }

    fn select(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        if let Ok(index) = self.get_action_index(element.handle, "select") {
            return self.do_atspi_action_by_index(&target, index);
        }
        // [P2] No "select" action: ask the parent's Selection interface.
        // Never ActionNotSupported here — consumers read that as a stale ref.
        let accessible =
            self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Accessible")?;
        let index: i32 = accessible
            .call("GetIndexInParent", &())
            .map_err(|e| self.call_failed(&target, "GetIndexInParent", e, None))?;
        let parent = self
            .get_atspi_parent(&target)
            .map_err(|e| self.stale_if_gone(&target).unwrap_or(e))?
            .ok_or_else(|| Error::Unsupported {
                feature: "select: the element has no parent to select it in".to_string(),
            })?;
        let selection =
            self.make_proxy(&parent.bus_name, &parent.path, "org.a11y.atspi.Selection")?;
        let selected: bool = selection.call("SelectChild", &(index,)).map_err(|e| {
            let unsupported = Error::Unsupported {
                feature: "select: the parent does not implement AT-SPI Selection".to_string(),
            };
            self.call_failed(&parent, "Selection.SelectChild", e, Some(unsupported))
        })?;
        if !selected {
            return Err(Error::Unsupported {
                feature: "the accessibility provider declined Selection.SelectChild".to_string(),
            });
        }
        Ok(())
    }

    fn expand(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let index = self
            .get_action_index(element.handle, "expand")
            .map_err(|_| Error::ActionNotSupported {
                action: "expand".to_string(),
                role: element.role,
            })?;
        self.do_atspi_action_by_index(&target, index)
    }

    fn collapse(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let index = self
            .get_action_index(element.handle, "collapse")
            .map_err(|_| Error::ActionNotSupported {
                action: "collapse".to_string(),
                role: element.role,
            })?;
        self.do_atspi_action_by_index(&target, index)
    }

    fn show_menu(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let index = self
            .get_action_index(element.handle, "show_menu")
            .map_err(|_| Error::ActionNotSupported {
                action: "show_menu".to_string(),
                role: element.role,
            })?;
        self.do_atspi_action_by_index(&target, index)
    }

    fn increment(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // Try stored AT-SPI2 action index first, fall back to Value interface
        if let Ok(index) = self.get_action_index(element.handle, "increment") {
            return self.do_atspi_action_by_index(&target, index);
        }
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Value")?;
        let current: f64 = proxy
            .get_property("CurrentValue")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Value.CurrentValue failed: {}", e),
            })?;
        let step: f64 = proxy.get_property("MinimumIncrement").unwrap_or(1.0);
        let step = if step <= 0.0 { 1.0 } else { step };
        proxy
            .set_property("CurrentValue", current + step)
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Value.SetCurrentValue failed: {}", e),
            })
    }

    fn decrement(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        if let Ok(index) = self.get_action_index(element.handle, "decrement") {
            return self.do_atspi_action_by_index(&target, index);
        }
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Value")?;
        let current: f64 = proxy
            .get_property("CurrentValue")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Value.CurrentValue failed: {}", e),
            })?;
        let step: f64 = proxy.get_property("MinimumIncrement").unwrap_or(1.0);
        let step = if step <= 0.0 { 1.0 } else { step };
        proxy
            .set_property("CurrentValue", current - step)
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Value.SetCurrentValue failed: {}", e),
            })
    }

    fn scroll_into_view(&self, element: &ElementData) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Component")?;
        proxy
            .call_method("ScrollTo", &(0u32,))
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("ScrollTo failed: {}", e),
            })?;
        Ok(())
    }

    // ── Window management ──────────────────────────────────────────
    //
    // Native X11/Sway window-manager requests handle the verbs they advertise.
    // On Wayland compositors without a native provider, AT-SPI still exposes
    // `activate` (Component.GrabFocus on the frame) and two geometry setters:
    // SetPosition and SetSize. There is no simulated-shortcut path.
    //
    // Window discovery is `App::windows` — `get_children(app)` filtered to
    // Window|Dialog — as on every platform.

    fn activate(&self, element: &ElementData) -> Result<()> {
        // The AT-SPI implementation of "activate this window" is the same
        // Component.GrabFocus path `focus` uses: the frame that receives
        // focus becomes the active window. No separate activate API exists.
        //
        // Implemented here rather than delegating to `focus` so a failure
        // reports the verb the caller requested — `activate` — not `focus`'s
        // name (the two share a mechanism, not an identity).
        //
        // A GrabFocus(false) reply is an honest "cannot activate this window",
        // reported as ActionNotSupported: the action-index fallback that
        // `focus` carries cannot fire for window-like roles, because
        // `action_indices` is populated only for roles in `role_has_actions`
        // (Window/Dialog excluded) and `map_atspi_action_name` maps no
        // "focus" action name. Unlike Windows/macOS, `activate` is deliberately
        // NOT advertised on AT-SPI — the adapters disagree about a frame
        // GrabFocus and no interface probe discriminates them (see the
        // reasons in `build_element_data`); advertised or not, the call here
        // and the `focus` call use the same mechanism, which is why the
        // deleted `focus` lookup would only ever pretend to a second path
        // (tenet 1). If GTK4's GrabFocus(false) is ever reproduced with an
        // actual focus-named Action interface entry (issue #98), the right
        // fix is a direct Action probe on this failure path, not a cache
        // lookup the role/name mapping rules can never fill.
        self.ensure_top_level_window_target(element, "activate")?;
        if self.window_manager.uses_native() {
            return self.window_manager.activate(element);
        }
        let target = self.get_cached(element.handle)?;
        let proxy = self
            .make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Component")
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Component proxy while activating failed: {}", e),
            })?;
        let reply = grab_focus_result(
            proxy.call_method("GrabFocus", &()),
            "activating",
            "activate",
            element.role,
        )?;
        let grabbed: bool = reply.body().deserialize().map_err(|e| Error::Platform {
            code: -1,
            message: format!("Component.GrabFocus reply while activating failed: {}", e),
        })?;
        if grabbed {
            return Ok(());
        }
        Err(Error::ActionNotSupported {
            action: "activate".to_string(),
            role: element.role,
        })
    }

    fn minimize(&self, element: &ElementData) -> Result<()> {
        self.ensure_top_level_window_target(element, "minimize")?;
        self.window_manager.minimize(element)
    }

    fn maximize(&self, element: &ElementData) -> Result<()> {
        self.ensure_top_level_window_target(element, "maximize")?;
        self.window_manager.maximize(element)
    }

    fn enter_fullscreen(&self, element: &ElementData) -> Result<()> {
        self.ensure_top_level_window_target(element, "enter_fullscreen")?;
        self.window_manager.enter_fullscreen(element)
    }

    fn restore(&self, element: &ElementData) -> Result<()> {
        self.ensure_top_level_window_target(element, "restore")?;
        self.window_manager.restore(element)
    }

    fn close(&self, element: &ElementData) -> Result<()> {
        self.ensure_top_level_window_target(element, "close")?;
        self.window_manager.close(element)
    }

    fn move_to(&self, element: &ElementData, x: i32, y: i32) -> Result<()> {
        self.ensure_top_level_window_target(element, "move_to")?;
        if self.window_manager.uses_native() {
            return self.window_manager.move_to(element, x, y);
        }
        let target = self.get_cached(element.handle)?;
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Component")?;
        let scale = crate::scale::coordinate_scale();
        let point = xa11y_core::Point::new(x, y).to_physical(scale);
        // coord_type 0 is screen coordinates, matching GetExtents above.
        let reply = proxy
            .call_method("SetPosition", &(point.x, point.y, 0u32))
            .map_err(|e| {
                if is_absent_member(&e) {
                    Error::ActionNotSupported {
                        action: "move_to".to_string(),
                        role: element.role,
                    }
                } else {
                    Error::Platform {
                        code: -1,
                        message: format!("Component.SetPosition while moving window failed: {e}"),
                    }
                }
            })?;
        let moved: bool = reply.body().deserialize().map_err(|e| Error::Platform {
            code: -1,
            message: format!("Component.SetPosition reply while moving window failed: {e}"),
        })?;
        if moved {
            Ok(())
        } else {
            Err(Error::ActionNotSupported {
                action: "move_to".to_string(),
                role: element.role,
            })
        }
    }

    fn resize_to(&self, element: &ElementData, w: u32, h: u32) -> Result<()> {
        self.ensure_top_level_window_target(element, "resize_to")?;
        if self.window_manager.uses_native() {
            return self.window_manager.resize_to(element, w, h);
        }
        let target = self.get_cached(element.handle)?;
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Component")?;
        let physical = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }
        .to_physical(crate::scale::coordinate_scale());
        let width = i32::try_from(physical.width).map_err(|_| Error::InvalidActionData {
            message: format!("resize width {w} is too large for AT-SPI"),
        })?;
        let height = i32::try_from(physical.height).map_err(|_| Error::InvalidActionData {
            message: format!("resize height {h} is too large for AT-SPI"),
        })?;
        let reply = proxy
            .call_method("SetSize", &(width, height))
            .map_err(|e| {
                if is_absent_member(&e) {
                    Error::ActionNotSupported {
                        action: "resize_to".to_string(),
                        role: element.role,
                    }
                } else {
                    Error::Platform {
                        code: -1,
                        message: format!("Component.SetSize while resizing window failed: {e}"),
                    }
                }
            })?;
        let resized: bool = reply.body().deserialize().map_err(|e| Error::Platform {
            code: -1,
            message: format!("Component.SetSize reply while resizing window failed: {e}"),
        })?;
        if resized {
            Ok(())
        } else {
            Err(Error::ActionNotSupported {
                action: "resize_to".to_string(),
                role: element.role,
            })
        }
    }

    fn set_value(&self, element: &ElementData, value: &str) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let proxy = self
            .make_proxy(
                &target.bus_name,
                &target.path,
                "org.a11y.atspi.EditableText",
            )
            .map_err(|_| Error::TextValueNotSupported)?;
        // [P3] One provider call per act: no DeleteText+InsertText fallback,
        // and a `false` reply is the provider declining, not success. An
        // absent `EditableText` (the Chrome URL bar only exposes read-only
        // `Text`; issue #101) is TextValueNotSupported unless the P1 probe
        // says the object itself is gone.
        let applied: bool = proxy.call("SetTextContents", &(value,)).map_err(|e| {
            self.call_failed(
                &target,
                "EditableText.SetTextContents",
                e,
                Some(Error::TextValueNotSupported),
            )
        })?;
        if !applied {
            return Err(Error::Unsupported {
                feature: "the accessibility provider declined SetTextContents".to_string(),
            });
        }
        Ok(())
    }

    fn set_numeric_value(&self, element: &ElementData, value: f64) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Value")?;
        proxy
            .set_property("CurrentValue", value)
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("SetValue failed: {}", e),
            })
    }

    fn type_text(&self, element: &ElementData, text: &str) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        // Insert text via EditableText interface (accessibility API, not input simulation).
        // Get cursor position from Text interface, then insert at that position.
        let text_proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Text");
        let insert_pos = text_proxy
            .as_ref()
            .ok()
            .and_then(|p| p.get_property::<i32>("CaretOffset").ok())
            .unwrap_or(-1); // -1 = append at end

        let proxy = self
            .make_proxy(
                &target.bus_name,
                &target.path,
                "org.a11y.atspi.EditableText",
            )
            .map_err(|_| Error::TextValueNotSupported)?;
        let pos = if insert_pos >= 0 {
            insert_pos
        } else {
            i32::MAX
        };
        proxy
            .call_method("InsertText", &(pos, text, text.len() as i32))
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("EditableText.InsertText failed: {}", e),
            })?;
        Ok(())
    }

    fn set_text_selection(&self, element: &ElementData, start: u32, end: u32) -> Result<()> {
        let target = self.get_cached(element.handle)?;
        let proxy = self.make_proxy(&target.bus_name, &target.path, "org.a11y.atspi.Text")?;
        proxy
            .call_method("SetSelection", &(0i32, start as i32, end as i32))
            .map_err(|e| Error::Platform {
                code: -1,
                message: format!("Text.SetSelection failed: {}", e),
            })?;
        Ok(())
    }

    fn perform_action(&self, element: &ElementData, action: &str) -> Result<()> {
        match action {
            "press" => self.press(element),
            "focus" => self.focus(element),
            "blur" => self.blur(element),
            "toggle" => self.toggle(element),
            "select" => self.select(element),
            "expand" => self.expand(element),
            "collapse" => self.collapse(element),
            "show_menu" => self.show_menu(element),
            "increment" => self.increment(element),
            "decrement" => self.decrement(element),
            "scroll_into_view" => self.scroll_into_view(element),
            "activate" => self.activate(element),
            "minimize" => self.minimize(element),
            "maximize" => self.maximize(element),
            "enter_fullscreen" => self.enter_fullscreen(element),
            "restore" => self.restore(element),
            "close" => self.close(element),
            // Payload verbs have no arguments on the generic escape hatch;
            // fail surfaceably with how to call them instead of guessing
            // (tenet 1: no silent fallback).
            "move_to" => Err(Error::InvalidActionData {
                message: "perform_action(\"move_to\") requires coordinates; call move_to(x, y)"
                    .to_string(),
            }),
            "resize_to" => Err(Error::InvalidActionData {
                message: "perform_action(\"resize_to\") requires dimensions; call \
                           resize_to(width, height)"
                    .to_string(),
            }),
            _ => Err(Error::ActionNotSupported {
                action: action.to_string(),
                role: element.role,
            }),
        }
    }

    fn subscribe(&self, element: &ElementData) -> Result<Subscription> {
        let pid = element.pid.ok_or(Error::Platform {
            code: -1,
            message: "Element has no PID for subscribe".to_string(),
        })?;
        let app_name = element.name.clone().unwrap_or_default();
        crate::events::subscribe_for_pid(self, pid, app_name)
    }
}

/// Whether a role typically has text or Value interface content.
/// Container/structural roles are skipped to save D-Bus round-trips.
fn role_has_value(role: Role) -> bool {
    !matches!(
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
    )
}

/// Whether a role typically supports actions via the Action interface.
/// Container and display-only roles are skipped to save D-Bus round-trips.
///
/// Window and Dialog are deliberately excluded: their semantic verbs do not
/// come from the Action interface. `activate` (the one Linux supports) is also
/// deliberately NOT advertised — see the note in `build_element_data` on why
/// (the AT-SPI adapters disagree about implementing a frame GrabFocus, so
/// advertising it would promise an action the call then declines); it stays
/// callable, and the state-mutating verbs are `Unsupported` on AT-SPI — so
/// admitting them here would only earn a pointless GetActions probe
/// returning no usable entries.
fn role_has_actions(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::RadioButton
            | Role::MenuItem
            | Role::Link
            | Role::ComboBox
            | Role::TextField
            | Role::TextArea
            | Role::SpinButton
            | Role::Tab
            | Role::TreeItem
            | Role::ListItem
            | Role::ScrollBar
            | Role::Slider
            | Role::Menu
            | Role::Image
            | Role::Unknown
    )
}

/// Classify a `Component.GrabFocus` call result, used by
/// [`focus`](LinuxProvider::focus) and [`activate`](LinuxProvider::activate).
///
/// The two verbs share the AT-SPI mechanism (there is no separate activate API),
/// and an adapter signals "this element cannot take focus" in one of two
/// honest shapes: a `false` reply (checked by both callers) or the D-Bus
/// `NotSupported` error, which is what the GTK4 adapter raises for
/// non-focusable controls instead of answering false (issue #98). Both mean
/// the adapter declined — `ActionNotSupported`, the documented contract that
/// the python/JS suites treat as xfail — and every other failure stays a
/// `Platform` error (tenet 1: no silent swallowing of unexpected failures).
fn grab_focus_result(
    call: std::result::Result<zbus::Message, zbus::Error>,
    verb: &str,
    action: &str,
    role: Role,
) -> Result<zbus::Message> {
    call.map_err(|e| match e {
        zbus::Error::MethodError(name, _, _) => {
            grab_focus_error(name.as_str().to_string(), verb, action, role)
        }
        other => Error::Platform {
            code: -1,
            message: format!("Component.GrabFocus while {verb} failed: {other}"),
        },
    })
}

/// Pure classification half of `grab_focus_result`, unit-testable without
/// constructing zbus internals.
fn grab_focus_error(name: String, verb: &str, action: &str, role: Role) -> Error {
    if name == "org.freedesktop.DBus.Error.NotSupported" {
        Error::ActionNotSupported {
            action: action.to_string(),
            role,
        }
    } else {
        Error::Platform {
            code: -1,
            message: format!("Component.GrabFocus while {verb} failed: {name}"),
        }
    }
}

/// Map AT-SPI2 role name to xa11y Role.
pub(crate) fn map_atspi_role(role_name: &str) -> Role {
    match role_name.to_lowercase().as_str() {
        "application" => Role::Application,
        "window" | "frame" => Role::Window,
        "dialog" | "file chooser" => Role::Dialog,
        "alert" | "notification" | "info bar" => Role::Alert,
        "push button" | "push button menu" => Role::Button,
        "toggle button" => Role::Switch,
        "check box" | "check menu item" => Role::CheckBox,
        "radio button" | "radio menu item" => Role::RadioButton,
        "entry" | "password text" => Role::TextField,
        "spin button" | "spinbutton" => Role::SpinButton,
        // "textbox" is the ARIA role name returned by WebKit2GTK for both
        // <input type="text"> and <textarea>.  Map to TextArea here so the
        // multi-line refinement below can downgrade single-line ones to TextField.
        // "embedded" is WebKit2GTK's numeric-path equivalent (role 78);
        // "terminal" is VTE — text is reachable via the Text interface.
        "text" | "textbox" | "embedded" | "terminal" => Role::TextArea,
        "label" | "static" | "caption" => Role::StaticText,
        "combo box" | "combobox" => Role::ComboBox,
        // "listbox" is the ARIA role name returned by WebKit2GTK for role="listbox".
        "list" | "list box" | "listbox" => Role::List,
        "list item" => Role::ListItem,
        "menu" | "popup menu" => Role::Menu,
        "menu item" | "tearoff menu item" => Role::MenuItem,
        "menu bar" => Role::MenuBar,
        "page tab" => Role::Tab,
        "page tab list" => Role::TabGroup,
        "table" | "tree table" => Role::Table,
        "table row" => Role::TableRow,
        // Bare "column header"/"row header" are WebKit's <th> and GTK
        // column-view headers; the "table …" pair is the classic toolkit one.
        "table cell"
        | "table column header"
        | "table row header"
        | "column header"
        | "row header" => Role::TableCell,
        "tool bar" => Role::Toolbar,
        "scroll bar" => Role::ScrollBar,
        "slider" => Role::Slider,
        "image" | "icon" | "desktop icon" => Role::Image,
        "link" => Role::Link,
        "panel" | "section" | "form" | "filler" | "viewport" | "scroll pane" | "paragraph"
        | "header" | "footer" | "grouping" | "block quote" | "redundant object" | "tree"
        | "comment" | "title bar" | "calendar" => Role::Group,
        "progress bar" | "level bar" => Role::ProgressBar,
        "tree item" => Role::TreeItem,
        "document web"
        | "document frame"
        | "document spreadsheet"
        | "document presentation"
        | "document text"
        | "document email" => Role::WebArea,
        "heading" => Role::Heading,
        "separator" => Role::Separator,
        "split pane" => Role::SplitGroup,
        "tooltip" | "tool tip" => Role::Tooltip,
        "status bar" | "statusbar" => Role::Status,
        "landmark" | "navigation" => Role::Navigation,
        _ => xa11y_core::unknown_role(role_name),
    }
}

/// `AtspiRole::Frame` — the numeric role a top-level window frame reports.
///
/// Named because `is_dock_frame` needs the number itself: the table below
/// folds `Frame` and `Window` (69) into the same `Role::Window`, and a panel
/// registers only as the former. `test_frame_role_number_is_the_atspi_frame_role`
/// pins it against the AtspiRole enum the tests carry.
const ATSPI_ROLE_FRAME: u32 = 23;

/// `AtspiRole::PasswordText`.
const ATSPI_ROLE_PASSWORD_TEXT: u32 = 40;

/// [P4] Whether an accessible is a password field, by `GetRoleName` or, when
/// that is empty, `GetRole`. Its text is a secret: callers must never read
/// `Text`/`Value` for it, so it never enters this process.
pub(crate) fn is_password_text(role_name: &str, role_num: u32) -> bool {
    role_name == "password text" || role_num == ATSPI_ROLE_PASSWORD_TEXT
}

/// Map AT-SPI2 numeric role (AtspiRole enum) to xa11y Role.
/// Values from atspi-common Role enum (repr(u32)).
pub(crate) fn map_atspi_role_number(role: u32) -> Role {
    match role {
        2 => Role::Alert,    // Alert
        5 => Role::Group,    // Calendar (mirrors UIA CalendarControlType)
        7 => Role::CheckBox, // CheckBox
        8 => Role::CheckBox, // CheckMenuItem
        // Bare ColumnHeader/RowHeader (10/47): WebKit and GTK emit these for
        // <th> / column-view header cells, distinct from the
        // TableColumnHeader/TableRowHeader pair (57/58) below.
        10 => Role::TableCell,   // ColumnHeader
        11 => Role::ComboBox,    // ComboBox
        13 => Role::Image,       // DesktopIcon
        16 => Role::Dialog,      // Dialog
        19 => Role::Dialog,      // FileChooser
        20 => Role::Group,       // Filler
        23 => Role::Window,      // Frame
        26 => Role::Image,       // Icon
        27 => Role::Image,       // Image
        29 => Role::StaticText,  // Label
        31 => Role::List,        // List
        32 => Role::ListItem,    // ListItem
        33 => Role::Menu,        // Menu
        34 => Role::MenuBar,     // MenuBar
        35 => Role::MenuItem,    // MenuItem
        37 => Role::Tab,         // PageTab
        38 => Role::TabGroup,    // PageTabList
        39 => Role::Group,       // Panel
        40 => Role::TextField,   // PasswordText
        41 => Role::Menu,        // PopupMenu
        42 => Role::ProgressBar, // ProgressBar
        43 => Role::Button,      // Button (push button)
        44 => Role::RadioButton, // RadioButton
        45 => Role::RadioButton, // RadioMenuItem
        47 => Role::TableCell,   // RowHeader
        48 => Role::ScrollBar,   // ScrollBar
        49 => Role::Group,       // ScrollPane
        50 => Role::Separator,   // Separator
        51 => Role::Slider,      // Slider
        52 => Role::SpinButton,  // SpinButton
        53 => Role::SplitGroup,  // SplitPane
        54 => Role::Status,      // StatusBar
        55 => Role::Table,       // Table
        56 => Role::TableCell,   // TableCell
        57 => Role::TableCell,   // TableColumnHeader
        58 => Role::TableCell,   // TableRowHeader
        59 => Role::MenuItem,    // TearoffMenuItem
        60 => Role::TextArea,    // Terminal (VTE and friends — text via the Text interface)
        61 => Role::TextArea,    // Text
        62 => Role::Switch,      // ToggleButton
        63 => Role::Toolbar,     // ToolBar
        64 => Role::Tooltip,     // ToolTip
        65 => Role::Group,       // Tree
        66 => Role::Table,       // TreeTable
        67 => Role::Unknown,     // Unknown
        68 => Role::Group,       // Viewport
        69 => Role::Window,      // Window
        // Text-document containers WebKit emits for plain markup: a bare
        // <p> (no ARIA naming) is Paragraph; header/footer/caption/block
        // quote come from the matching HTML elements. Containers normalize
        // to Group, caption to StaticText, mirroring the by-name map.
        71 => Role::Group,       // Header
        72 => Role::Group,       // Footer
        73 => Role::Group,       // Paragraph
        75 => Role::Application, // Application
        78 => Role::TextArea, // Embedded — WebKit2GTK uses this for <input type="text"> and <textarea>;
        // multi-line refinement below downgrades single-line ones to TextField
        79 => Role::TextField,  // Entry
        81 => Role::StaticText, // Caption
        82 => Role::WebArea,    // DocumentFrame
        83 => Role::Heading,    // Heading
        85 => Role::Group,      // Section
        86 => Role::Group,      // RedundantObject
        87 => Role::Group,      // Form
        88 => Role::Link,       // Link
        90 => Role::TableRow,   // TableRow
        91 => Role::TreeItem,   // TreeItem
        // Document roots (92-96): LibreOffice exposes spreadsheet /
        // presentation / text / email documents with these; all normalize to
        // WebArea alongside DocumentFrame/DocumentWeb above.
        92 => Role::WebArea,      // DocumentSpreadsheet
        93 => Role::WebArea,      // DocumentPresentation
        94 => Role::WebArea,      // DocumentText
        95 => Role::WebArea,      // DocumentWeb
        96 => Role::WebArea,      // DocumentEmail
        97 => Role::Group,        // Comment (ARIA comment container)
        98 => Role::List,         // ListBox — WebKit2GTK emits this for <ul role="listbox">
        99 => Role::Group,        // Grouping
        101 => Role::Alert,       // Notification
        102 => Role::Alert,       // InfoBar (GtkInfoBar message strip)
        103 => Role::ProgressBar, // LevelBar (GtkLevelBar)
        104 => Role::Group,       // TitleBar (mirrors UIA TitleBar)
        105 => Role::Group,       // BlockQuote
        110 => Role::Navigation,  // Landmark
        116 => Role::StaticText,  // Static
        129 => Role::Button,      // PushButtonMenu
        _ => xa11y_core::unknown_role(&format!("AT-SPI role number {role}")),
    }
}

/// Depth cap for the GTK press-fallback BFS. Wrapper patterns nest at most
/// two levels (e.g. `AdwSplitButton` → inner toggle-button → inner label);
/// depth 3 covers them with headroom without letting the walk wander.
const GTK_FALLBACK_MAX_DEPTH: u32 = 3;

/// Hard cap on visited accessibles per fallback resolution. Defensive — the
/// depth cap already bounds typical cases to ≤ 20 nodes.
const GTK_FALLBACK_MAX_NODES: usize = 200;

/// Whether an AT-SPI2 role name represents an activatable widget we are
/// willing to invoke via the fallback path. Deliberately narrow: roles that
/// could carry destructive or misleading semantics (e.g. `label` with the
/// synthesised clipboard/selection actions) are excluded.
fn is_actionable_atspi_role(role: &str) -> bool {
    matches!(
        role,
        "push button"
            | "toggle button"
            | "check box"
            | "radio button"
            | "menu item"
            | "check menu item"
            | "radio menu item"
            | "link"
            | "page tab"
            | "tab"
            | "list item"
            | "tree item"
    )
}

/// Roles the BFS refuses to descend into. Static and decorative roles never
/// lead anywhere useful, and `label` in particular carries a fan-out of
/// text-editing actions (`clipboard.copy`, `selection.delete`, …) that we
/// must never reach via a press heuristic.
fn is_never_descend_atspi_role(role: &str) -> bool {
    matches!(
        role,
        "label" | "separator" | "image" | "icon" | "static" | "caption"
    )
}

/// Whether a top-level accessible is a desktop panel or dock: an AT-SPI2
/// `frame` (numeric role [`ATSPI_ROLE_FRAME`]) whose attribute set carries
/// `window-type:dock`.
///
/// The role arrives as the number `GetRole` reports rather than the
/// `GetRoleName` string: the numeric method is the one every AT-SPI object
/// implements. `map_atspi_role_number` is no help here — `Frame` (23) and
/// `Window` (69) both map to `Role::Window`, and only the former is what a
/// panel registers as.
///
/// `window-type` mirrors the window manager's `_NET_WM_WINDOW_TYPE` hint, so
/// it is set by the panel itself rather than inferred — an ordinary
/// application window carries `normal`, or no `window-type` at all.
fn is_dock_frame(role_number: u32, attributes: &HashMap<String, String>) -> bool {
    role_number == ATSPI_ROLE_FRAME && attributes.get("window-type").is_some_and(|t| t == "dock")
}

/// Classify the structured reply before rendering it. `UnknownObject` means
/// this exact accessible was destroyed, not that the peer timed out or that
/// `GetChildren` is unimplemented. Direct reads retain the stale-element error;
/// live traversals prune it through `live_atspi_children`.
fn children_call_error(err: zbus::Error, aref: &AccessibleRef) -> Error {
    match &err {
        zbus::Error::MethodError(name, ..)
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownObject" =>
        {
            Error::ElementStale {
                selector: format!("{}{}", aref.bus_name, aref.path),
            }
        }
        _ => Error::Platform {
            code: -1,
            message: format!("GetChildren failed: {err}"),
        },
    }
}

/// D-Bus error names that mean the member was never there, as opposed to the
/// call having failed.
///
/// A reply of `UnknownMethod` / `UnknownInterface` / `UnknownObject` is the
/// bus answering the question: this object does not implement that. AT-SPI
/// bridges implement the interface partially — AccessKit's serves `GetRole`
/// and `GetLocalizedRoleName` but no `GetRoleName` at all — so a caller that
/// reads absence as failure fails on a perfectly healthy desktop.
const ABSENT_MEMBER_ERRORS: [&str; 3] = [
    "org.freedesktop.DBus.Error.UnknownMethod",
    "org.freedesktop.DBus.Error.UnknownInterface",
    "org.freedesktop.DBus.Error.UnknownObject",
];

/// D-Bus error names that mean the *peer* is gone, as opposed to the call
/// having failed.
///
/// The shell scan reads every top-level of every application on the bus, and an
/// application that quits between `list_apps()` and those reads answers with
/// one of these. That is genuine absence — the process and its surfaces are
/// gone — not a failure to hide (tenet 1): there is nothing left to report and
/// nothing a caller could do about it. A hung or broken application is a
/// different outcome and stays a failure: it answers `NoReply` or `Timeout`, or
/// does not answer at all.
///
/// Kept separate from [`ABSENT_MEMBER_ERRORS`] because the two are different
/// axes — "this object does not implement that member" versus "this bus name
/// has no owner any more".
const PEER_GONE_ERRORS: [&str; 2] = [
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NameHasNoOwner",
];

/// Whether a failed D-Bus call means the peer left the bus (see
/// [`PEER_GONE_ERRORS`]).
///
/// Matches the structured error name for the same reason
/// [`is_absent_member`] does, never a substring of a formatted message.
fn is_peer_gone(err: &zbus::Error) -> bool {
    match err {
        zbus::Error::MethodError(name, ..) => PEER_GONE_ERRORS.contains(&name.as_str()),
        _ => false,
    }
}

/// Whether a failed D-Bus call means the member is absent (see
/// [`ABSENT_MEMBER_ERRORS`]) rather than that the call failed.
///
/// `Proxy::call_method` surfaces an error *reply* as
/// `zbus::Error::MethodError`, which keeps the error name structured — so
/// this matches the name itself, never a substring of a formatted message.
/// Every other `zbus::Error` (no reply, a timeout, a service that left the
/// bus, a broken connection) is a genuine failure and stays one.
fn is_absent_member(err: &zbus::Error) -> bool {
    match err {
        zbus::Error::MethodError(name, ..) => ABSENT_MEMBER_ERRORS.contains(&name.as_str()),
        _ => false,
    }
}

/// Re-word a classification failure so it names the top-level that would not
/// answer and the application that had just listed it.
///
/// The shell scan classifies a frame by two D-Bus reads, and either failing
/// aborts the listing rather than being read as "not a panel" (tenet 1). What
/// the raw `GetRole failed: …` does not say is *whose* window it was — which
/// is the whole of what a consumer needs to know to go look (tenet 6).
fn classify_failure(app: &ElementData, top: &AccessibleRef, err: Error) -> Error {
    let name = app.name.as_deref().unwrap_or("<unnamed>");
    let pid = app.pid.map(|p| format!(" pid={p}")).unwrap_or_default();
    Error::Platform {
        code: match &err {
            Error::Platform { code, .. } => *code,
            _ => -1,
        },
        message: format!(
            "could not classify top-level {} of application \"{name}\"{pid} as a shell \
             surface: {err}",
            top.path
        ),
    }
}

/// Map an AT-SPI2 action name to its canonical `snake_case` xa11y action name.
///
/// Toolkit-specific aliases are normalised to the single canonical name:
///   "click" / "activate" / "press" / "invoke" / "dodefault" → "press"
///   "toggle" / "check" / "uncheck"            → "toggle"
///   "expand" / "open"                          → "expand"
///   "collapse" / "close"                       → "collapse"
///   "menu" / "showmenu" / "showcontextmenu" / "popup" / "show menu" → "show_menu"
///   "select"                                    → "select"
///   "increment"                                 → "increment"
///   "decrement"                                 → "decrement"
///
/// Returns `None` for unrecognised names.
fn map_atspi_action_name(action_name: &str) -> Option<String> {
    // Normalise by lowercasing and stripping underscores/spaces so that
    // "show_menu", "show menu", "showMenu" and "showContextMenu" all collapse
    // to the same canonical form. Chromium is the main motivator: it uses
    // `doDefault` as the default activation action on ~190 elements per
    // window (Back/Forward buttons, toolbar icons, menu items, sliders,
    // notifications) and `showContextMenu` as the context-menu action. The
    // previous table dropped both, leaving those elements with no `press`
    // mapping at all. See issue #101.
    let lower = action_name.to_lowercase();
    let collapsed: String = lower.chars().filter(|c| !matches!(c, '_' | ' ')).collect();
    let canonical = match collapsed.as_str() {
        "click" | "activate" | "press" | "invoke" | "dodefault" => "press",
        "toggle" | "check" | "uncheck" => "toggle",
        "expand" | "open" => "expand",
        "collapse" | "close" => "collapse",
        "select" => "select",
        "menu" | "showmenu" | "showcontextmenu" | "contextmenu" | "popup" => "show_menu",
        "increment" => "increment",
        "decrement" => "decrement",
        _ => return None,
    };
    Some(canonical.to_string())
}

/// Resolve AT-SPI's overloaded `menu` role using positive activation evidence.
///
/// GTK 3 reports both popup menu containers and submenu-bearing entries as
/// `menu`. Its entries implement Action with `click`, while the GtkMenu
/// container does not implement Action. Parentage and children cannot separate
/// the two: GTK reparents the popup under its owning entry and exposes submenu
/// items directly as children of that same entry.
fn refine_menu_role(role: Role, actions: &[String]) -> Role {
    if role == Role::Menu && actions.iter().any(|action| action == "press") {
        Role::MenuItem
    } else {
        role
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_mapping() {
        assert_eq!(map_atspi_role("push button"), Role::Button);
        assert_eq!(map_atspi_role("toggle button"), Role::Switch);
        assert_eq!(map_atspi_role("check box"), Role::CheckBox);
        assert_eq!(map_atspi_role("entry"), Role::TextField);
        assert_eq!(map_atspi_role("label"), Role::StaticText);
        assert_eq!(map_atspi_role("window"), Role::Window);
        assert_eq!(map_atspi_role("frame"), Role::Window);
        assert_eq!(map_atspi_role("dialog"), Role::Dialog);
        assert_eq!(map_atspi_role("combo box"), Role::ComboBox);
        assert_eq!(map_atspi_role("slider"), Role::Slider);
        assert_eq!(map_atspi_role("panel"), Role::Group);
        assert_eq!(map_atspi_role("unknown_thing"), Role::Unknown);
    }

    #[test]
    fn menu_with_native_activation_is_a_menu_item() {
        assert_eq!(
            refine_menu_role(Role::Menu, &["press".to_string()]),
            Role::MenuItem
        );
    }

    #[test]
    fn non_activatable_menu_stays_a_container() {
        assert_eq!(refine_menu_role(Role::Menu, &[]), Role::Menu);
        assert_eq!(
            refine_menu_role(Role::Menu, &["show_menu".to_string()]),
            Role::Menu
        );
    }

    #[test]
    fn activation_does_not_change_other_roles() {
        assert_eq!(
            refine_menu_role(Role::MenuBar, &["press".to_string()]),
            Role::MenuBar
        );
        assert_eq!(
            refine_menu_role(Role::MenuItem, &["press".to_string()]),
            Role::MenuItem
        );
    }

    #[test]
    fn test_numeric_role_mapping() {
        // ToggleButton (62) must map to Switch, not Button.
        // GTK4's Gtk.Switch and Gtk.ToggleButton both report numeric role 62.
        assert_eq!(map_atspi_role_number(62), Role::Switch);
        // Sanity-check a few well-established numeric mappings.
        assert_eq!(map_atspi_role_number(43), Role::Button); // PushButton
        assert_eq!(map_atspi_role_number(7), Role::CheckBox);
        assert_eq!(map_atspi_role_number(67), Role::Unknown); // AT-SPI Unknown
    }

    #[test]
    fn grab_focus_notsupported_is_action_not_supported() {
        // The GTK4 adapter answers a button GrabFocus with the D-Bus
        // NotSupported error instead of a false reply (issue #98); the
        // classifier must surface it as ActionNotSupported so the suites'
        // xfail contract holds, and keep everything else a platform error
        // (tenet 1).
        let declined = grab_focus_error(
            "org.freedesktop.DBus.Error.NotSupported".into(),
            "focusing",
            "focus",
            Role::Button,
        );
        match declined {
            Error::ActionNotSupported { action, role } => {
                assert_eq!(action, "focus");
                assert_eq!(role, Role::Button);
            }
            other => panic!("expected ActionNotSupported, got {other:?}"),
        }

        let broken = grab_focus_error(
            "org.freedesktop.DBus.Error.UnknownMethod".into(),
            "activating",
            "activate",
            Role::Window,
        );
        match broken {
            Error::Platform { code, .. } => assert_eq!(code, -1),
            other => panic!("expected Platform error, got {other:?}"),
        }
    }

    /// The full AtspiRole enum (atspi 2.52) as (number, GetRoleName-style
    /// name) pairs. The name form is the enum nick with dashes replaced by
    /// spaces, which is what providers hand back over D-Bus.
    const ATSPI_ROLE_ENUM: &[(u32, &str)] = &[
        (0, "invalid"),
        (1, "accelerator label"),
        (2, "alert"),
        (3, "animation"),
        (4, "arrow"),
        (5, "calendar"),
        (6, "canvas"),
        (7, "check box"),
        (8, "check menu item"),
        (9, "color chooser"),
        (10, "column header"),
        (11, "combo box"),
        (12, "date editor"),
        (13, "desktop icon"),
        (14, "desktop frame"),
        (15, "dial"),
        (16, "dialog"),
        (17, "directory pane"),
        (18, "drawing area"),
        (19, "file chooser"),
        (20, "filler"),
        (21, "focus traversable"),
        (22, "font chooser"),
        (23, "frame"),
        (24, "glass pane"),
        (25, "html container"),
        (26, "icon"),
        (27, "image"),
        (28, "internal frame"),
        (29, "label"),
        (30, "layered pane"),
        (31, "list"),
        (32, "list item"),
        (33, "menu"),
        (34, "menu bar"),
        (35, "menu item"),
        (36, "option pane"),
        (37, "page tab"),
        (38, "page tab list"),
        (39, "panel"),
        (40, "password text"),
        (41, "popup menu"),
        (42, "progress bar"),
        (43, "push button"),
        (44, "radio button"),
        (45, "radio menu item"),
        (46, "root pane"),
        (47, "row header"),
        (48, "scroll bar"),
        (49, "scroll pane"),
        (50, "separator"),
        (51, "slider"),
        (52, "spin button"),
        (53, "split pane"),
        (54, "status bar"),
        (55, "table"),
        (56, "table cell"),
        (57, "table column header"),
        (58, "table row header"),
        (59, "tearoff menu item"),
        (60, "terminal"),
        (61, "text"),
        (62, "toggle button"),
        (63, "tool bar"),
        (64, "tool tip"),
        (65, "tree"),
        (66, "tree table"),
        (67, "unknown"),
        (68, "viewport"),
        (69, "window"),
        (70, "extended"),
        (71, "header"),
        (72, "footer"),
        (73, "paragraph"),
        (74, "ruler"),
        (75, "application"),
        (76, "autocomplete"),
        (77, "editbar"),
        (78, "embedded"),
        (79, "entry"),
        (80, "chart"),
        (81, "caption"),
        (82, "document frame"),
        (83, "heading"),
        (84, "page"),
        (85, "section"),
        (86, "redundant object"),
        (87, "form"),
        (88, "link"),
        (89, "input method window"),
        (90, "table row"),
        (91, "tree item"),
        (92, "document spreadsheet"),
        (93, "document presentation"),
        (94, "document text"),
        (95, "document web"),
        (96, "document email"),
        (97, "comment"),
        (98, "list box"),
        (99, "grouping"),
        (100, "image map"),
        (101, "notification"),
        (102, "info bar"),
        (103, "level bar"),
        (104, "title bar"),
        (105, "block quote"),
        (106, "audio"),
        (107, "video"),
        (108, "definition"),
        (109, "article"),
        (110, "landmark"),
        (111, "log"),
        (112, "marquee"),
        (113, "math"),
        (114, "rating"),
        (115, "timer"),
        (116, "static"),
        (117, "math fraction"),
        (118, "math root"),
        (119, "subscript"),
        (120, "superscript"),
        (121, "description list"),
        (122, "description term"),
        (123, "description value"),
        (124, "footnote"),
        (125, "content deletion"),
        (126, "content insertion"),
        (127, "mark"),
        (128, "suggestion"),
        (129, "push button menu"),
    ];

    #[test]
    fn numeric_and_name_role_maps_agree() {
        // The provider resolves roles by number on some paths (resolve_role)
        // and by name on others (build_element_data). If the two maps drift,
        // the same element reports different roles depending on how it was
        // reached — this happened for StatusBar (54, name-only), ToolTip
        // (64, and 93 was mis-numbered as Tooltip), and Landmark (110).
        // Sweep the whole enum so any future addition lands in both maps.
        for &(num, name) in ATSPI_ROLE_ENUM {
            let by_num = map_atspi_role_number(num);
            let by_name = map_atspi_role(name);
            assert_eq!(
                by_num, by_name,
                "AT-SPI role {num} ({name:?}) diverges: numeric map gives \
                 {by_num:?}, name map gives {by_name:?}"
            );
        }
    }

    #[test]
    fn test_action_name_mapping() {
        assert_eq!(map_atspi_action_name("click"), Some("press".to_string()));
        assert_eq!(map_atspi_action_name("activate"), Some("press".to_string()));
        assert_eq!(map_atspi_action_name("press"), Some("press".to_string()));
        assert_eq!(map_atspi_action_name("invoke"), Some("press".to_string()));
        // Chromium uses `doDefault` for default activation on ~190 elements.
        assert_eq!(
            map_atspi_action_name("doDefault"),
            Some("press".to_string())
        );
        assert_eq!(
            map_atspi_action_name("do_default"),
            Some("press".to_string())
        );
        assert_eq!(map_atspi_action_name("toggle"), Some("toggle".to_string()));
        assert_eq!(map_atspi_action_name("check"), Some("toggle".to_string()));
        assert_eq!(map_atspi_action_name("uncheck"), Some("toggle".to_string()));
        assert_eq!(map_atspi_action_name("expand"), Some("expand".to_string()));
        assert_eq!(map_atspi_action_name("open"), Some("expand".to_string()));
        assert_eq!(
            map_atspi_action_name("collapse"),
            Some("collapse".to_string())
        );
        assert_eq!(map_atspi_action_name("close"), Some("collapse".to_string()));
        assert_eq!(map_atspi_action_name("select"), Some("select".to_string()));
        assert_eq!(map_atspi_action_name("menu"), Some("show_menu".to_string()));
        assert_eq!(
            map_atspi_action_name("showmenu"),
            Some("show_menu".to_string())
        );
        assert_eq!(
            map_atspi_action_name("popup"),
            Some("show_menu".to_string())
        );
        assert_eq!(
            map_atspi_action_name("show menu"),
            Some("show_menu".to_string())
        );
        // Chrome / Chromium expose the URL-bar context-menu action as
        // `showContextMenu`; the previous table missed both spellings.
        assert_eq!(
            map_atspi_action_name("showContextMenu"),
            Some("show_menu".to_string())
        );
        assert_eq!(
            map_atspi_action_name("show_context_menu"),
            Some("show_menu".to_string())
        );
        assert_eq!(
            map_atspi_action_name("increment"),
            Some("increment".to_string())
        );
        assert_eq!(
            map_atspi_action_name("decrement"),
            Some("decrement".to_string())
        );
        assert_eq!(map_atspi_action_name("foobar"), None);
    }

    /// All known AT-SPI2 aliases map to one of the well-known action names,
    /// and re-mapping the canonical name produces the same canonical name.
    #[test]
    fn test_action_name_aliases_roundtrip() {
        let atspi_names = [
            "click",
            "activate",
            "press",
            "invoke",
            "doDefault",
            "do_default",
            "toggle",
            "check",
            "uncheck",
            "expand",
            "open",
            "collapse",
            "close",
            "select",
            "menu",
            "showmenu",
            "showContextMenu",
            "show_context_menu",
            "popup",
            "show menu",
            "increment",
            "decrement",
        ];
        for name in atspi_names {
            let canonical = map_atspi_action_name(name).unwrap_or_else(|| {
                panic!("AT-SPI2 name {:?} should map to a canonical name", name)
            });
            // Re-mapping the canonical name must produce itself.
            let back = map_atspi_action_name(&canonical)
                .unwrap_or_else(|| panic!("canonical {:?} should map back to itself", canonical));
            assert_eq!(
                canonical, back,
                "AT-SPI2 {:?} -> {:?} -> {:?} (expected {:?})",
                name, canonical, back, canonical
            );
        }
    }

    /// Case-insensitive mapping works.
    #[test]
    fn test_action_name_case_insensitive() {
        assert_eq!(map_atspi_action_name("Click"), Some("press".to_string()));
        assert_eq!(map_atspi_action_name("TOGGLE"), Some("toggle".to_string()));
        assert_eq!(
            map_atspi_action_name("Increment"),
            Some("increment".to_string())
        );
    }

    /// The GTK press-fallback's actionable-role set must include the inner
    /// toggle-button pattern used by `GtkMenuButton` / `AdwMenuButton`, plus
    /// the other standard activatable roles we're willing to synthesise a
    /// click for.
    #[test]
    fn test_gtk_fallback_actionable_roles() {
        for role in [
            "push button",
            "toggle button",
            "check box",
            "radio button",
            "menu item",
            "link",
            "tab",
            "list item",
            "tree item",
        ] {
            assert!(
                is_actionable_atspi_role(role),
                "{role:?} should be actionable"
            );
        }
    }

    /// Never treat static / decorative accessibles as fallback candidates.
    /// Particularly important for `label`, whose synthesised text-editing
    /// actions (`clipboard.copy`, `selection.delete`) must never be invoked
    /// by a press heuristic.
    #[test]
    fn test_gtk_fallback_non_actionable_roles() {
        for role in [
            "label",
            "panel",
            "filler",
            "section",
            "group",
            "image",
            "separator",
            "static",
            "frame",
            "window",
        ] {
            assert!(
                !is_actionable_atspi_role(role),
                "{role:?} must not be treated as actionable"
            );
        }
    }

    /// The BFS stops at static/decorative roles. `label` in particular must
    /// never be descended into — its children are text spans with bogus
    /// actions.
    #[test]
    fn test_gtk_fallback_never_descend_roles() {
        for role in ["label", "separator", "image", "icon", "static", "caption"] {
            assert!(
                is_never_descend_atspi_role(role),
                "{role:?} must block BFS descent"
            );
        }
        // Containers — panel / filler / section / group / frame — stay
        // walkable so the BFS can reach a wrapped inner actionable.
        for role in ["panel", "filler", "section", "group", "frame"] {
            assert!(
                !is_never_descend_atspi_role(role),
                "container role {role:?} must remain descendable"
            );
        }
    }

    fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A panel frame is the `window-type:dock` attribute and nothing else —
    /// xfce4-panel's rows are unnamed frames, so no name test can stand in
    /// for it.
    #[test]
    fn test_dock_frame_detection() {
        assert!(is_dock_frame(
            ATSPI_ROLE_FRAME,
            &attrs(&[("toolkit", "GTK"), ("window-type", "dock")])
        ));
        // An ordinary application window: same role, different window type.
        assert!(!is_dock_frame(
            ATSPI_ROLE_FRAME,
            &attrs(&[("window-type", "normal")])
        ));
        // Most toolkits set no `window-type` at all.
        assert!(!is_dock_frame(
            ATSPI_ROLE_FRAME,
            &attrs(&[("toolkit", "GTK")])
        ));
        assert!(!is_dock_frame(ATSPI_ROLE_FRAME, &HashMap::new()));
    }

    /// The frame role number is the one `GetRoleName` would spell "frame" —
    /// the classification moved to `GetRole` because AccessKit's bridge
    /// implements only the numeric method.
    #[test]
    fn test_frame_role_number_is_the_atspi_frame_role() {
        let (number, name) = ATSPI_ROLE_ENUM
            .iter()
            .find(|(_, name)| *name == "frame")
            .copied()
            .expect("the AtspiRole table must carry `frame`");
        assert_eq!(number, ATSPI_ROLE_FRAME, "{name} moved");
    }

    /// The dock attribute only makes a surface on a top-level frame. `panel`
    /// (39) is the AT-SPI role for an ordinary grouping container (it maps to
    /// `Role::Group`), and `window` (69) / `dialog` (16) are not what panels
    /// register as — `Window` in particular shares `Role::Window` with
    /// `Frame`, so only the raw number tells them apart.
    #[test]
    fn test_dock_frame_requires_frame_role() {
        let dock = attrs(&[("window-type", "dock")]);
        for (number, name) in ATSPI_ROLE_ENUM
            .iter()
            .filter(|(number, _)| *number != ATSPI_ROLE_FRAME)
        {
            assert!(
                !is_dock_frame(*number, &dock),
                "role {number} ({name:?}) must not be reported as a shell surface"
            );
        }
    }

    /// Build the `zbus::Error` a D-Bus error reply of `name` arrives as, the
    /// way `Proxy::call_method` hands it back.
    fn method_error(name: &str) -> zbus::Error {
        let call = zbus::Message::method_call("/org/a11y/atspi/accessible/7", "GetAttributes")
            .expect("method-call builder")
            .serial(std::num::NonZeroU32::new(1).expect("nonzero"))
            .build(&())
            .expect("build method call");
        let reply = zbus::Message::error(&call.header(), name)
            .expect("error builder")
            .serial(std::num::NonZeroU32::new(2).expect("nonzero"))
            .build(&"boom")
            .expect("build error reply");
        zbus::Error::from(reply)
    }

    #[test]
    fn children_errors_classify_only_unknown_object_as_stale() {
        let aref = AccessibleRef {
            bus_name: ":1.42".into(),
            path: "/org/a11y/atspi/accessible/7".into(),
        };
        let err = children_call_error(
            method_error("org.freedesktop.DBus.Error.UnknownObject"),
            &aref,
        );
        assert!(matches!(err, Error::ElementStale { selector }
            if selector.contains(&aref.bus_name) && selector.contains(&aref.path)));
        for name in [
            "org.freedesktop.DBus.Error.UnknownMethod",
            "org.freedesktop.DBus.Error.UnknownInterface",
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.Timeout",
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ] {
            assert!(
                matches!(children_call_error(method_error(name), &aref),
                Error::Platform { message, .. } if message.contains(name)),
                "{name}"
            );
        }
        // Text resembling an error name is never enough to prune a subtree.
        assert!(matches!(
            children_call_error(
                zbus::Error::Failure("org.freedesktop.DBus.Error.UnknownObject".into()),
                &aref
            ),
            Error::Platform { .. }
        ));
    }

    /// Minimal AT-SPI peer: the first child disappears when its children are
    /// read, while a later sibling stays live. Exercises the real D-Bus reply
    /// path deterministically instead of relying on a scheduling race in Qt.
    struct LiveTreePeer {
        name: &'static str,
        role: &'static str,
        children: Vec<&'static str>,
        children_error: Option<&'static str>,
    }

    #[zbus::interface(name = "org.a11y.atspi.Accessible")]
    impl LiveTreePeer {
        fn get_children(
            &self,
        ) -> zbus::fdo::Result<Vec<(String, zbus::zvariant::OwnedObjectPath)>> {
            if let Some(error) = self.children_error {
                return Err(match error {
                    "UnknownObject" => zbus::fdo::Error::UnknownObject("destroyed".into()),
                    "NoReply" => zbus::fdo::Error::NoReply("unresponsive".into()),
                    _ => panic!("unsupported test error"),
                });
            }
            Ok(self
                .children
                .iter()
                .map(|path| {
                    (
                        "org.test.LiveTree".into(),
                        zbus::zvariant::OwnedObjectPath::try_from(*path).expect("test path"),
                    )
                })
                .collect())
        }

        fn get_role_name(&self) -> &str {
            self.role
        }

        #[zbus(property)]
        fn name(&self) -> &str {
            self.name
        }
    }

    fn live_tree_peer(
        error: &'static str,
        vanished_role: &'static str,
    ) -> (Connection, std::sync::Arc<LinuxProvider>, ElementData) {
        let (server_socket, client_socket) =
            std::os::unix::net::UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            zbus::blocking::connection::Builder::async_io_unix_stream(server_socket)
                .server(zbus::Guid::generate())
                .expect("server GUID")
                .p2p()
                .serve_at(
                    "/root",
                    LiveTreePeer {
                        name: "App",
                        role: "application",
                        children: vec!["/vanished", "/survivor"],
                        children_error: None,
                    },
                )
                .expect("root interface")
                .serve_at(
                    "/vanished",
                    LiveTreePeer {
                        name: "Vanished",
                        role: vanished_role,
                        children: vec![],
                        children_error: Some(error),
                    },
                )
                .expect("vanished interface")
                .serve_at(
                    "/survivor",
                    LiveTreePeer {
                        name: "Survivor",
                        role: "push button",
                        children: vec![],
                        children_error: None,
                    },
                )
                .expect("survivor interface")
                .build()
                .expect("server connection")
        });
        let connection = zbus::blocking::connection::Builder::async_io_unix_stream(client_socket)
            .p2p()
            .build()
            .expect("client connection");
        let server = server.join().expect("server thread");
        let provider = std::sync::Arc::new(LinuxProvider {
            a11y_bus: connection,
            handle_cache: Mutex::new(HashMap::new()),
            action_indices: Mutex::new(HashMap::new()),
            window_manager: WindowManager::new(),
        });
        let root_ref = AccessibleRef {
            bus_name: "org.test.LiveTree".into(),
            path: "/root".into(),
        };
        let root = provider.build_element_data(&root_ref, None);
        (server, provider, root)
    }

    #[test]
    fn dump_tree_and_locators_survive_a_vanished_subtree() {
        let (_server, provider, root) = live_tree_peer("UnknownObject", "panel");
        let app = xa11y_core::App::from_data(provider.clone(), root);
        let tree = app.tree(None).expect("tree prunes vanished descendants");
        assert!(tree
            .children
            .iter()
            .any(|child| child.name.as_deref() == Some("Survivor")));
        let dump = app.dump(None).expect("dump survives UnknownObject");
        assert!(dump.contains("Survivor"));
        assert_eq!(
            app.locator("button")
                .elements()
                .expect("locator survives")
                .len(),
            1
        );
        let vanished = app
            .children()
            .expect("children")
            .into_iter()
            .find(|child| child.name.as_deref() == Some("Vanished"))
            .expect("vanished snapshot");
        assert!(vanished
            .children()
            .expect("destroyed node has no live children")
            .is_empty());
        assert!(matches!(
            provider.get_atspi_children(&AccessibleRef {
                bus_name: "org.test.LiveTree".into(),
                path: "/vanished".into(),
            }),
            Err(Error::ElementStale { .. })
        ));
    }

    #[test]
    fn nested_application_disappearance_does_not_drop_live_siblings() {
        let (_server, provider, root) = live_tree_peer("UnknownObject", "application");
        let app = xa11y_core::App::from_data(provider, root);
        let dump = app.dump(None).expect("nested application pruned");
        assert!(dump.contains("Survivor"));
        assert!(!dump.contains("Vanished"));
    }

    #[test]
    fn dump_and_tree_preserve_real_children_failures() {
        let (_server, provider, root) = live_tree_peer("NoReply", "panel");
        let app = xa11y_core::App::from_data(provider, root);
        for result in [app.tree(None).map(|_| ()), app.dump(None).map(|_| ())] {
            assert!(matches!(result, Err(Error::Platform { message, .. })
                if message.contains("NoReply")));
        }
    }

    /// An object that does not implement the method it was asked for is
    /// absence, not failure: AccessKit's AT-SPI bridge (egui and friends)
    /// answers `GetRole` but has no `GetRoleName`, and reading that reply as a
    /// platform error failed the whole shell listing for every desktop running
    /// one such app.
    #[test]
    fn an_unimplemented_member_reads_as_absence() {
        for name in ABSENT_MEMBER_ERRORS {
            assert!(
                is_absent_member(&method_error(name)),
                "{name} must read as absence"
            );
        }
    }

    /// Everything else stays a failure — including the D-Bus errors that look
    /// superficially similar but mean the far end is broken or gone.
    /// An application that quits between `list_apps()` and the shell scan's
    /// reads answers `ServiceUnknown`. That is the process being gone, not a
    /// failure worth aborting a whole-desktop listing for — and it used to
    /// abort it, making `ShellSurface::list()` nondeterministic on a busy
    /// desktop. A hung application is a different outcome and still fails.
    #[test]
    fn a_departed_peer_is_absence_not_failure() {
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ] {
            assert!(
                is_peer_gone(&method_error(name)),
                "{name} means the peer left"
            );
            // The two axes stay separate: a departed peer is not a statement
            // about which members an object implements.
            assert!(!is_absent_member(&method_error(name)), "{name}");
        }
    }

    #[test]
    fn a_hung_or_broken_peer_is_still_a_failure() {
        for name in [
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.Timeout",
            "org.freedesktop.DBus.Error.Disconnected",
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.DBus.Error.UnknownMethod",
        ] {
            assert!(
                !is_peer_gone(&method_error(name)),
                "{name} must not be read as the peer having left"
            );
        }
        assert!(!is_peer_gone(&zbus::Error::InvalidReply));
    }

    #[test]
    fn other_dbus_failures_are_not_absence() {
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.Timeout",
            "org.freedesktop.DBus.Error.Disconnected",
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.DBus.Error.UnknownProperty",
        ] {
            assert!(
                !is_absent_member(&method_error(name)),
                "{name} must stay a failure"
            );
        }
        // A transport-level failure carries no error name at all.
        assert!(!is_absent_member(&zbus::Error::InvalidReply));
    }

    /// A frame the scan could not classify must fail the listing, naming the
    /// app and the object path. `Panel` is the only kind Linux vends, so
    /// swallowing this would render a broken panel process as a desktop with
    /// no panels at all — indistinguishable from the honest empty answer.
    #[test]
    fn a_classification_failure_names_the_app_and_the_top_level() {
        let mut app = ElementData::for_role(Role::Application);
        app.name = Some("xfce4-panel".to_string());
        app.pid = Some(4242);
        let top = AccessibleRef {
            bus_name: ":1.42".to_string(),
            path: "/org/a11y/atspi/accessible/7".to_string(),
        };
        let err = classify_failure(
            &app,
            &top,
            Error::Platform {
                code: -1,
                message: "GetRole failed: connection closed".to_string(),
            },
        );
        let Error::Platform { code, message } = err else {
            panic!("a D-Bus failure must stay a platform error");
        };
        assert_eq!(code, -1);
        assert!(message.contains("xfce4-panel"), "{message}");
        assert!(message.contains("pid=4242"), "{message}");
        assert!(
            message.contains("/org/a11y/atspi/accessible/7"),
            "{message}"
        );
        // Never masks the underlying D-Bus error (tenet 1).
        assert!(message.contains("GetRole failed"), "{message}");
    }

    /// An unnamed application still produces a usable message — the object
    /// path is what a consumer actually goes looking with.
    #[test]
    fn a_classification_failure_survives_an_unnamed_app() {
        let app = ElementData::for_role(Role::Application);
        let top = AccessibleRef {
            bus_name: ":1.9".to_string(),
            path: "/org/a11y/atspi/accessible/3".to_string(),
        };
        let err = classify_failure(
            &app,
            &top,
            Error::Platform {
                code: -1,
                message: "GetAttributes failed".to_string(),
            },
        );
        let msg = err.to_string();
        assert!(msg.contains("<unnamed>"), "{msg}");
        assert!(msg.contains("/org/a11y/atspi/accessible/3"), "{msg}");
    }

    // ── Vendored patches P1-P4 (see PATCHES.md) ─────────────────────────────
    //
    // One p2p peer per test. Each path serves only the interfaces its case
    // needs; a path that serves nothing answers `UnknownObject`, a served path
    // without the interface answers `UnknownInterface` — zbus's ObjectServer
    // replies exactly like a real AT-SPI bridge whose object was destroyed.

    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    type PeerBuilder = zbus::blocking::connection::Builder<'static>;

    const PEER: &str = "org.test.Patch";

    fn peer_ref(path: &str) -> AccessibleRef {
        AccessibleRef {
            bus_name: PEER.into(),
            path: path.into(),
        }
    }

    fn patch_peer(
        serve: impl FnOnce(PeerBuilder) -> zbus::Result<PeerBuilder> + Send + 'static,
    ) -> (Connection, LinuxProvider) {
        let (server_socket, client_socket) =
            std::os::unix::net::UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            let builder = zbus::blocking::connection::Builder::async_io_unix_stream(server_socket)
                .server(zbus::Guid::generate())
                .expect("server GUID")
                .p2p();
            serve(builder)
                .expect("serve objects")
                .build()
                .expect("server connection")
        });
        let connection = zbus::blocking::connection::Builder::async_io_unix_stream(client_socket)
            .p2p()
            .build()
            .expect("client connection");
        let server = server.join().expect("server thread");
        let provider = LinuxProvider {
            a11y_bus: connection,
            handle_cache: Mutex::new(HashMap::new()),
            action_indices: Mutex::new(HashMap::new()),
            window_manager: WindowManager::new(),
        };
        (server, provider)
    }

    struct PatchAccessible {
        /// `None`: `GetRole` fails with `Failed` whose *message* names
        /// `UnknownObject` — a translated-message stand-in that must not count.
        role: Option<u32>,
        role_name: &'static str,
        index_in_parent: i32,
        parent: &'static str,
        children: Vec<&'static str>,
    }

    impl PatchAccessible {
        fn new(role: u32, role_name: &'static str) -> Self {
            Self {
                role: Some(role),
                role_name,
                index_in_parent: 0,
                parent: "/org/a11y/atspi/null",
                children: vec![],
            }
        }

        fn child_of(mut self, parent: &'static str, index: i32) -> Self {
            self.parent = parent;
            self.index_in_parent = index;
            self
        }
    }

    #[zbus::interface(name = "org.a11y.atspi.Accessible")]
    impl PatchAccessible {
        fn get_role(&self) -> zbus::fdo::Result<u32> {
            self.role.ok_or_else(|| {
                zbus::fdo::Error::Failed("org.freedesktop.DBus.Error.UnknownObject".into())
            })
        }

        fn get_role_name(&self) -> &str {
            self.role_name
        }

        fn get_index_in_parent(&self) -> i32 {
            self.index_in_parent
        }

        fn get_children(&self) -> Vec<(String, zbus::zvariant::OwnedObjectPath)> {
            self.children
                .iter()
                .map(|path| {
                    (
                        PEER.into(),
                        zbus::zvariant::OwnedObjectPath::try_from(*path).expect("test path"),
                    )
                })
                .collect()
        }

        #[zbus(property)]
        fn parent(&self) -> (String, zbus::zvariant::OwnedObjectPath) {
            (
                PEER.into(),
                zbus::zvariant::OwnedObjectPath::try_from(self.parent).expect("test path"),
            )
        }
    }

    /// `DoAction` answers `reply`, or fails with `Failed` when `None`.
    struct PatchAction {
        reply: Option<bool>,
        calls: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.a11y.atspi.Action")]
    impl PatchAction {
        fn do_action(&self, _index: i32) -> zbus::fdo::Result<bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.reply
                .ok_or_else(|| zbus::fdo::Error::Failed("the toolkit refused".into()))
        }
    }

    /// Records every `SelectChild` index; declines index 9.
    struct PatchSelection {
        selected: Arc<Mutex<Vec<i32>>>,
    }

    #[zbus::interface(name = "org.a11y.atspi.Selection")]
    impl PatchSelection {
        fn select_child(&self, index: i32) -> bool {
            self.selected.lock().expect("selected").push(index);
            index != 9
        }
    }

    struct PatchEditable {
        reply: bool,
    }

    #[zbus::interface(name = "org.a11y.atspi.EditableText")]
    impl PatchEditable {
        fn set_text_contents(&self, _text: &str) -> bool {
            self.reply
        }
    }

    /// Serves the secret and counts every read of it.
    struct PatchText {
        reads: Arc<AtomicUsize>,
    }

    #[zbus::interface(name = "org.a11y.atspi.Text")]
    impl PatchText {
        fn get_text(&self, _start: i32, _end: i32) -> String {
            self.reads.fetch_add(1, Ordering::SeqCst);
            "hunter2".into()
        }

        #[zbus(property)]
        fn character_count(&self) -> i32 {
            self.reads.fetch_add(1, Ordering::SeqCst);
            7
        }
    }

    #[test]
    fn p1_failed_action_probes_get_role_by_error_name() {
        let calls = Arc::new(AtomicUsize::new(0));
        let action = |calls: &Arc<AtomicUsize>| PatchAction {
            reply: None,
            calls: calls.clone(),
        };
        let (a, b, c) = (action(&calls), action(&calls), action(&calls));
        let (_server, provider) = patch_peer(move |peer| {
            peer.serve_at("/refused", PatchAccessible::new(43, "push button"))?
                .serve_at("/refused", a)?
                .serve_at("/no_accessible", b)?
                .serve_at(
                    "/translated",
                    PatchAccessible {
                        role: None,
                        ..PatchAccessible::new(43, "push button")
                    },
                )?
                .serve_at("/translated", c)
        });
        // Nothing served: DoAction and the GetRole probe both answer UnknownObject.
        let err = provider
            .do_atspi_action_by_index(&peer_ref("/dead"), 0)
            .unwrap_err();
        assert!(
            matches!(&err, Error::ElementStale { selector } if selector.ends_with("/dead")),
            "{err:?}"
        );
        // The probe answering UnknownInterface is also a dead object.
        assert!(matches!(
            provider.do_atspi_action_by_index(&peer_ref("/no_accessible"), 0),
            Err(Error::ElementStale { .. })
        ));
        // A live object that refuses keeps today's Platform error.
        assert!(matches!(
            provider.do_atspi_action_by_index(&peer_ref("/refused"), 0),
            Err(Error::Platform { message, .. }) if message.starts_with("DoAction(0) failed")
        ));
        // A probe failure whose message (not name) says UnknownObject is not stale.
        assert!(matches!(
            provider.do_atspi_action_by_index(&peer_ref("/translated"), 0),
            Err(Error::Platform { .. })
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn p2_select_without_an_action_uses_the_parents_selection() {
        let selected = Arc::new(Mutex::new(Vec::new()));
        let action_calls = Arc::new(AtomicUsize::new(0));
        let selection = PatchSelection {
            selected: selected.clone(),
        };
        let action = PatchAction {
            reply: Some(true),
            calls: action_calls.clone(),
        };
        let (_server, provider) = patch_peer(move |peer| {
            peer.serve_at("/list", PatchAccessible::new(31, "list"))?
                .serve_at("/list", selection)?
                .serve_at("/plain", PatchAccessible::new(31, "list"))?
                .serve_at(
                    "/item",
                    PatchAccessible::new(32, "list item").child_of("/list", 2),
                )?
                .serve_at(
                    "/declined",
                    PatchAccessible::new(32, "list item").child_of("/list", 9),
                )?
                .serve_at(
                    "/unselectable",
                    PatchAccessible::new(32, "list item").child_of("/plain", 1),
                )?
                .serve_at(
                    "/actionable",
                    PatchAccessible::new(32, "list item").child_of("/list", 4),
                )?
                .serve_at("/actionable", action)
        });
        let element = |path: &str| provider.build_element_data(&peer_ref(path), None);

        provider
            .select(&element("/item"))
            .expect("selected via parent");
        assert_eq!(*selected.lock().unwrap(), vec![2]);

        assert!(matches!(
            provider.select(&element("/declined")),
            Err(Error::Unsupported { feature }) if feature.contains("declined Selection.SelectChild")
        ));
        assert!(matches!(
            provider.select(&element("/unselectable")),
            Err(Error::Unsupported { feature }) if feature.contains("does not implement")
        ));
        assert!(matches!(
            provider.select(&element("/gone")),
            Err(Error::ElementStale { .. })
        ));

        // A cached "select" action keeps the DoAction path.
        let actionable = element("/actionable");
        provider.action_indices.lock().unwrap().insert(
            actionable.handle,
            HashMap::from([("select".to_string(), 0)]),
        );
        provider.select(&actionable).expect("selected via DoAction");
        assert_eq!(action_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*selected.lock().unwrap(), vec![2, 9]);
    }

    #[test]
    fn p3_set_value_is_one_call_and_honours_the_reply() {
        let (_server, provider) = patch_peer(|peer| {
            peer.serve_at("/field", PatchAccessible::new(79, "entry"))?
                .serve_at("/field", PatchEditable { reply: true })?
                .serve_at("/declining", PatchAccessible::new(79, "entry"))?
                .serve_at("/declining", PatchEditable { reply: false })?
                .serve_at("/readonly", PatchAccessible::new(79, "entry"))
        });
        let element = |path: &str| provider.build_element_data(&peer_ref(path), None);

        provider
            .set_value(&element("/field"), "new")
            .expect("SetTextContents true");
        // `false` is a refusal, not success, and there is no fallback call.
        assert!(matches!(
            provider.set_value(&element("/declining"), "new"),
            Err(Error::Unsupported { feature })
                if feature == "the accessibility provider declined SetTextContents"
        ));
        // No EditableText on a live object: unchanged classification.
        assert!(matches!(
            provider.set_value(&element("/readonly"), "new"),
            Err(Error::TextValueNotSupported)
        ));
        // Dead object: the P1 probe wins over TextValueNotSupported.
        assert!(matches!(
            provider.set_value(&element("/gone"), "new"),
            Err(Error::ElementStale { .. })
        ));
    }

    #[test]
    fn p4_password_text_is_never_read() {
        let secret_reads = Arc::new(AtomicUsize::new(0));
        let entry_reads = Arc::new(AtomicUsize::new(0));
        let (password_text, unnamed_text, entry_text) = (
            PatchText {
                reads: secret_reads.clone(),
            },
            PatchText {
                reads: secret_reads.clone(),
            },
            PatchText {
                reads: entry_reads.clone(),
            },
        );
        let (_server, provider) = patch_peer(move |peer| {
            peer.serve_at(
                "/app",
                PatchAccessible {
                    children: vec!["/password", "/unnamed", "/entry"],
                    ..PatchAccessible::new(75, "application")
                },
            )?
            .serve_at("/password", PatchAccessible::new(40, "password text"))?
            .serve_at("/password", password_text)?
            // No role name: the numeric role alone (raw "role_num:40") decides.
            .serve_at("/unnamed", PatchAccessible::new(40, ""))?
            .serve_at("/unnamed", unnamed_text)?
            .serve_at("/entry", PatchAccessible::new(79, "entry"))?
            .serve_at("/entry", entry_text)
        });

        let password = provider.build_element_data(&peer_ref("/password"), None);
        assert_eq!(password.value, None);
        assert_eq!(password.raw["atspi_role"], "password text");
        let unnamed = provider.build_element_data(&peer_ref("/unnamed"), None);
        assert_eq!(unnamed.value, None);
        assert_eq!(unnamed.raw["atspi_role"], "role_num:40");
        // Control: the same Text interface is read for an ordinary entry.
        let entry = provider.build_element_data(&peer_ref("/entry"), None);
        assert_eq!(entry.value.as_deref(), Some("hunter2"));

        // The selector fast path (`matches_ref`) and event snapshots too.
        let snapshot =
            crate::events::build_event_snapshot(&provider.a11y_bus, &peer_ref("/password"), None)
                .expect("event snapshot");
        assert_eq!(snapshot.value, None);
        let root = provider.build_element_data(&peer_ref("/app"), None);
        let app = xa11y_core::App::from_data(Arc::new(provider), root);
        let matches = app
            .locator(r#"[value="hunter2"]"#)
            .elements()
            .expect("locator");
        assert_eq!(matches.len(), 1, "only the ordinary entry matches");

        assert_eq!(secret_reads.load(Ordering::SeqCst), 0);
        assert!(entry_reads.load(Ordering::SeqCst) > 0);
    }
}
