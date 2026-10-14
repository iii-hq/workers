# Vendored xa11y-linux 0.15.2: patches

Base: crates.io `xa11y-linux` 0.15.2 (MIT, `LICENSE` fetched from the
upstream `v0.15.2` tag; the crates.io tarball lacks it).

Re-applying on an upgrade: start from the new pristine crate, hand-copy this
`Cargo.toml`, redo every item below, then run the checks at the end. Patch
sites in the source are marked `[P1]` … `[P4]`; the new tests sit under
`// ── Vendored patches P1-P4` at the end of `src/atspi.rs`.

## Goals

- Keep only the AT-SPI semantic provider (`LinuxProvider`): `atspi.rs`,
  `events.rs`, `window.rs` (X11 EWMH), `scale.rs` (`Xft.dpi`), `session.rs`.
- Link no system C library: pure Rust (x11rb `RustConnection`, zbus).

## Removals

| What | Why |
|------|-----|
| `src/wayland_input.rs` | uinput input backend; pulled `evdev` + `xkbcommon` |
| `src/input.rs` | the consumer has its own X11 input driver |
| `src/screenshot.rs` | portal + `GetImage` capture; pulled `png` (direct dep) |
| `window.rs`: Sway backend (`SwayWindowBackend`, `SwayCandidate`, `sway_*` fns, `ServerIdentity::Sway`, `WaylandGlobals`, `advertised_wlr_foreign_toplevel_version`, Sway IPC consts, 6 Sway tests) | pulled `nix` + `wayland-client`. `WindowManager::new` now maps a Wayland session straight to `WindowBackend::Unavailable("no native Wayland window-management backend")` without connecting to anything; every `WindowBackend::Sway` match arm is deleted |
| `WindowFacts::protocol_version` and the `window_protocol_version` raw key in `atspi.rs` | only the Sway backend ever set it |
| `scale.rs`: `screenshot_scale`, `wayland_output_scale*`, `OutputInfo` + its `Dispatch` impls, `combine_wayland_scale`, `sane_scale`, their tests and `wayland_mock_tests` | pulled `wayland-protocols`/`wayland-client`; `screenshot_scale` was used only by `screenshot.rs`. Module doc rewritten; `coordinate_scale` unchanged |
| `session.rs`: the `XA11Y_LINUX_*_BACKEND` overrides | `select_backend(capability)` takes only the capability. Deleted the `_ if capability == "input simulation" => Wayland` arm, the `"wayland" \| "uinput"` arm and all explicit-override arms. Tests: kept pure/mixed (plus a no-endpoint case) and stale-session; dropped `explicit_overrides_require_their_endpoint`, `invalid_override_is_actionable` |
| `atspi.rs` `connect_a11y_bus`: silent fallback to the session bus | a broken a11y bus now fails: `Error::Platform { code: -1, message: "AT-SPI accessibility bus unavailable (<step>): <bus error>" }` for step `session bus`, `org.a11y.Bus.GetAddress`, `bus address` or `connect`. No app text is involved. `events.rs` subscriptions use the same function |
| `lib.rs` | module doc rewritten; exports only `LinuxProvider` |
| `stub.rs` | only the `LinuxProvider` stub (no `LinuxInputProvider` / `LinuxScreenshot`) |
| `tests/` | all three were input/Wayland smoke tests; no `[[test]]` entries |
| `Cargo.toml` | hand-written (no workspace inheritance, own empty `[workspace]`, `publish = false`). Dropped `evdev`, `xkbcommon`, `png`, `nix`, `wayland-client`, `wayland-protocols`, dev `wayland-server`, dev `x11rb`, and x11rb's `xtest` feature (nothing left uses XTest; `window.rs`/`scale.rs` need only core `xproto`) |

## Behaviour patches

### P1: dead objects are `ElementStale`, by D-Bus error name

`atspi.rs` adds `LinuxProvider::stale_if_gone(aref)` and
`LinuxProvider::call_failed(aref, call, err, if_absent)`. After a failed
call, `stale_if_gone` calls `org.a11y.atspi.Accessible.GetRole` on the same
bus name and object path. If that fails with `UnknownObject`, `UnknownMethod` or
`UnknownInterface` (the existing `is_absent_member`, which matches the
structured error *name* and never the message, because GDBus translates
messages), the result is `Error::ElementStale { selector: "<bus><path>" }`.
Any other probe outcome keeps the caller's error.

Applied at:
- `do_atspi_action_by_index`, the single `DoAction` choke point (callers:
  press, toggle, select, expand, collapse, show_menu, increment, decrement):
  a D-Bus error becomes stale or `Platform "DoAction(i) failed: …"` as
  before. A `false` reply is unchanged.
- `set_value` (P3) and the P2 `select` path.

### P2: `select()` without a "select" action uses the parent's `Selection`

If the element has a cached "select" action, the `DoAction` path stays as it
was. Otherwise the code calls `Accessible.GetIndexInParent` on the element,
then `Accessible.Parent`, then `org.a11y.atspi.Selection.SelectChild(index)`
on the parent.
- `false` reply → `Error::Unsupported { feature: "the accessibility provider declined Selection.SelectChild" }`.
  It is never `ActionNotSupported`, because the consumer reads an advertised
  `ActionNotSupported` as a stale ref.
- If the parent probes as dead (P1), or the element itself is dead
  (`GetIndexInParent` / `Parent` failure plus the probe), the result is `ElementStale`.
- A live parent without `Selection` gives `Error::Unsupported { feature: "select: the parent does not implement AT-SPI Selection" }`.
- No parent gives `Error::Unsupported { feature: "select: the element has no parent to select it in" }`.
- Upstream 0.15.2 never called `GetIndexInParent`, `SelectChild` or anything
  on `org.a11y.atspi.Selection`. xa11y has no typed proxies: every call is a
  string-named `zbus::blocking::Proxy::call`.

### P3: `set_value()` makes one provider call and respects the reply

`EditableText.SetTextContents` is the only call. A `false` reply gives
`Error::Unsupported { feature: "the accessibility provider declined SetTextContents" }`.
The `DeleteText` + `InsertText` fallback is deleted. On a D-Bus error:
`ElementStale` if P1 says the object is gone, else `TextValueNotSupported`
for `UnknownMethod`/`UnknownInterface`/`UnknownObject` (upstream matched
`"UnknownMethod"`/`"UnknownInterface"` as substrings of the *message*; this
copy matches the name), else `Platform "EditableText.SetTextContents failed: …"`.

### P4: password text never enters the process

`atspi.rs` adds `is_password_text(role_name, role_num)`, which is true for
`"password text"` or `ATSPI_ROLE_PASSWORD_TEXT` (40). For such an element the
`Text` and `Value` interfaces are never read. The guard sits at every reader:
- `build_element_data`: skips `get_value`, so `value` and `raw.atspi_value` are `None`.
  `raw.atspi_role` is unchanged (`"password text"` / `"role_num:40"`).
- `matches_ref` `value` filter (the selector fast path): calls `GetRole` first
  and fails closed (no read) when the role is 40 or `GetRole` fails.
- `events.rs` `build_event_snapshot`: skips `get_value`. The function is now
  `pub(crate)` so the test can reach it.

Known ceiling: an event subscription (`subscribe`) still receives AT-SPI
`TextChanged` signal bodies, whose `any_data` can carry inserted text.
`dispatch` drops it (`_any`), but zbus has already read the bytes. Not
subscribing is the only way to avoid that.

## New tests (`src/atspi.rs`, hermetic zbus p2p peers)

- `p1_failed_action_probes_get_role_by_error_name`
- `p2_select_without_an_action_uses_the_parents_selection`
- `p3_set_value_is_one_call_and_honours_the_reply`
- `p4_password_text_is_never_read`

## Checks after re-applying

```sh
cargo test --locked --lib
cargo clippy --locked --all-targets -- -D warnings
cargo tree -e normal --target x86_64-unknown-linux-gnu   # no xkbcommon/evdev/nix/wayland-*/xcb
```

`png` still appears in the tree, through `xa11y-core` 0.15.2, which depends on
it unconditionally for `Screenshot` encoding. It is pure Rust
(flate2/miniz_oxide) and links no C library.
