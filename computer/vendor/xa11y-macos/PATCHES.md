# Vendored xa11y-macos 0.15.2: patches

Base: crates.io `xa11y-macos` 0.15.2 (MIT, `LICENSE` from the upstream
`v0.15.2` tag; the crates.io tarball lacks it). `Cargo.lock` is the tarball's.

Re-applying on an upgrade: start from the new pristine crate, hand-copy this
`Cargo.toml`, redo every item below, then run the checks at the end. Patch
sites in the source are marked `[M1]` … `[M4]`; the new tests sit under
`// ── Vendored patches M1-M4` at the end of `src/ax.rs`.

## Goals

- The whole crate, unchanged apart from M1-M4, so a re-apply is a plain diff.
- Close the cases where the provider reports a write as done although AppKit
  dropped it, or hands a secure field's value over because a read failed.

## Removals

None. `Cargo.toml` is hand-written (no workspace inheritance, own empty
`[workspace]`, `publish = false`); its dependencies equal upstream's, so no
crate enters the consumer's graph.

## Why settability is not enough

AppKit answers `kAXErrorSuccess` to an attribute write whose setter
`isAccessibilitySelectorAllowed:` refuses: it skips the setter and drops the
value. `AXUIElementIsAttributeSettable` answers `true` for any
`NSAccessibilityElement` subclass that overrides the setter, whatever that
method says, so it tells whether the class implements the setter, not whether
this element accepts it. AccessKit's `PlatformNode` overrides both
`setAccessibilitySelected:` and `setAccessibilityValue:` (accesskit_macos
0.27.1 `node.rs`). Measured on macOS 27.2 with in-process probes.

## Behaviour patches

### M1: `select()` needs an observed `AXSelected`

If the snapshot's `raw` has no `AXSelected` key (the attribute was absent when
observed; `raw` keeps only attributes that were read), `select()` returns
`Error::Unsupported { feature: "select: the element exposed no AXSelected when
observed" }` before any IPC. Otherwise it writes `AXSelected = true` as
before (after M3).

- `Unsupported`, never `ActionNotSupported`: the consumer reads an advertised
  verb's `ActionNotSupported` as a removed element.
- Covers every AccessKit node that is not item-like (buttons, fields, check
  boxes, tabs) and items whose selection lives only on the container (Qt).
- Ceiling: an exposed but inert `AXSelected` (an AccessKit item that is not
  clickable and selectable) still dispatches unverified. No AX query predicts
  it.

### M2: `set_value()` asks whether `AXValue` is settable

After M3, `set_value()` calls the crate's own `is_attr_settable(AXValue)`
(already used by `expand`/`collapse`). `false` gives `Error::Unsupported {
feature: "set_value: AXValue is not settable on this element" }`; a failed
probe gives `Platform "IsAttributeSettable(AXValue) failed"`.

- Closes the case for native AppKit: a non-editable `NSTextField` (read as
  `AXStaticText`) answers not-settable and would drop the write.
- WebKit (Safari) and Chromium (Chrome) text fields, combo boxes and address
  bars answer settable, checked on macOS 27.2.
- Ceiling: AccessKit nodes always answer settable (see above), so a read-only
  AccessKit field still dispatches unverified, and AppKit drops the write.
  `actions` and `states.editable` stay role-based, as upstream.

### M3: acts re-read `AXRole` before they dispatch

`MacOSProvider::live_cached(element)` replaces `get_cached` in `press`,
`toggle`, `select` and `set_value`. It reads `AXRole` once, keeping the
AXError (`raw_string(read_raw_attr(..))`), and `check_live` returns
`Error::ElementStale { selector: "handle:<n>" }` when `role_says_stale`:

- `kAXErrorInvalidUIElement`, or
- a definite answer (including "no role") that differs from the snapshot's
  non-empty `raw["ax_role"]`.

Busy (`-25204`), malformed answers and an empty snapshot role are
inconclusive and dispatch as before. Why: AppKit acknowledges `AXPress` and
attribute writes on an element that ignores them, so the verb's own error
code cannot tell a detached element; a detached AccessKit node answers
`AXRole` with no value. The `press` actions gate and the `toggle` role gate
stay in front, so a refusal still costs no IPC.

- Cost: one extra IPC per act. On a hung app the probe and the action can
  each wait out the AX messaging timeout.
- Ceiling: role identity only. A detached but retained native view keeps its
  role and passes; an app that changes an element's role in place reads as
  stale (nothing is dispatched; a new find recovers).

### M4: an unanswered `AXSubrole` is reported, never folded into "absent"

Upstream turns both "no subrole" and "the subrole read failed" into `None`,
so a secure field whose subrole read failed looked like a plain field.

- `exception_safe.m`: `safe_ax_batch_slot_error(slot)` classifies one
  `AXUIElementCopyMultipleAttributeValues` slot as an AXError: `0` for a value,
  `kAXErrorNoValue` for `kCFNull`, the carried code for an `AXValue` of
  `kAXValueAXErrorType`, `-1` for a missing slot (short array), `-9999` for an
  exception.
- `BatchAttrs::raw(idx)` maps it the way `read_raw_attr` maps one read:
  unsupported / no value is `Absent`, any other code `Unanswered(code)`.
- `ResolvedAttrs.subrole_error: Option<i32>`, filled by `from_batch` and
  `from_individual` through `raw_string` (a non-string subrole is `-1`).
- `build_snapshot_data` inserts `raw["ax_subrole_error"] = <code>` when it is
  set. The consumer (computer `walk.rs` `is_secret`) treats that key as a
  secret, failing closed.

AppKit answers a missing subrole as `-25212` (no value), which stays absent:
ordinary text fields keep their values.

## New tests (`mod vendored_patches` in `src/ax.rs`)

No TCC grant needed: the provider is built bare and its elements are null.

- `m1_select_needs_an_observed_ax_selected`: refused before the cache is
  consulted; with `AXSelected` observed it reaches the (empty) cache.
- `m2_set_value_asks_before_it_sets`: the settability probe fails before the
  set; with Accessibility granted, the system-wide element is refused.
- `m3_acts_reread_the_role_before_dispatch`: the `role_says_stale` table;
  every verb reads an attribute before dispatch (upstream reads none);
  `check_live` turns a role-less answer into `ElementStale` only against an
  observed role; with Accessibility granted, `live_cached` on the
  system-wide element observed as a button is `ElementStale` end to end.
- `m4_an_unanswered_subrole_is_not_absent`: slot classification for a string,
  `-25204`, `-25205`, `-25212`, `kCFNull`, a number and a missing slot.

## Checks

On a Mac with Accessibility and Screen Recording granted to the terminal:

    cargo test --locked --lib
    cargo clippy --locked --all-targets -- -D warnings

then the consumer's `semantic_live` e2e (computer `architecture/internals.md`,
Tests). CI runs only `cargo test --locked --lib vendored_patches`: the
upstream tests that build a provider need TCC.
