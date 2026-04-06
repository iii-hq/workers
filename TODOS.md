# TODOs

## SDK: Add push notifications for trigger types and workers

**What:** Add `on_trigger_types_available` and `on_workers_available` callbacks to all three SDKs (Node, Rust, Python), matching the existing `on_functions_available` pattern.

**Why:** The LSP worker needs live updates for trigger types and workers but can only get push updates for functions. Currently works around this by re-fetching `list_trigger_types()` and `list_workers()` whenever `on_functions_available` fires. The clean fix is symmetric push events at the SDK level.

**Context:** Discovered during /plan-eng-review of the iii-lsp design. The engine already has internal trigger types `engine::functions-available` and `engine::workers-available`, but only `functions-available` is exposed as a convenience method in the SDKs. Adding `on_trigger_types_available` would require a new engine trigger type and SDK wrappers in all three languages.

**Depends on:** Engine changes to add a `engine::trigger-types-available` trigger type. This is engine-repo work, not workers-repo work.

**Pros:** Cleaner LSP implementation, benefits any future tooling that needs live registry data.
**Cons:** Requires engine + 3 SDK changes. Non-trivial coordination across repos.
