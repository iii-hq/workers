# Isolated Sentinel UI checks

These fixtures mount the real Sentinel list/dialog and real Console primitives. Only the iii RPC host is replaced, with synthetic data. No engine, live catalog, investigation, Compose, or active Sentinel binary is contacted or rebuilt.

Use an existing monorepo checkout with its workspace dependencies already available, including `ade/web/node_modules`, root `node_modules`, and `sentinel/ui/node_modules`. All package paths resolve relative to `import.meta.url`; dependency symlinks are resolved for Vite's filesystem allowlist. These runners do not install anything. If dependencies or Chromium are unavailable, stop and report that prerequisite; do not install them as part of these checks.

From `sentinel/ui`:

```sh
node --test src/page/*.test.mjs src/settings/*.test.mjs
node node_modules/typescript/bin/tsc --noEmit
node build.mjs
node fixtures/sp-selector-after.mjs
node fixtures/smoke.mjs
```

Chromium uses the existing Playwright browser cache by default. To select an already-installed executable, set `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` to its absolute executable path. No machine-specific default is embedded in these runners. Vite binds only `127.0.0.1:4197`; each runner closes its browser and server in `finally`. Run the two browser scripts sequentially.

`sp-selector-data.ts` is a synthetic-only catalog: one unbroken 3500-character description, long names/IDs, multiple scrollable entries, Default, and an invalid-inheritance entry. `sp-selector-after.mjs` verifies actual listbox and option bounds, complete text, normal pointer selection, disabled entries, native keyboard/focus and cancel at 1440×1000, 390×844, and a 520px pane in a 1440×1000 viewport, dark/light with reduced motion. `smoke.mjs` covers preserved investigation behavior. No image bytes pass through model context. The runners write new `cr-after*` and `cr-regression*` artifacts, preserving previous captures. Scripts write these artifacts under `sentinel/docs/evidence/group-investigation-a73c/`.

## Before provenance

Historical before/after screenshots are available at the immutable [screenshot revision](https://github.com/iii-hq/workers/tree/4196a08c6825f8e830659b078e92a38d37d58faf/sentinel/docs/screenshots/group-investigation). They were captured from the real components with synthetic catalog data before the documentation artifacts were removed from this checkout. The baseline runner and baseline JSON are not versioned in this tree; do not assume they are available locally or recreate a before capture using corrected source.

The current runners capture the corrected components and report automated bounds and interaction checks. Pixel review is a separate activity and must not be inferred from successful screenshot creation.

Both runners create their output directory before entering the guarded run. `SENTINEL_UI_EVIDENCE_DIR` can select a separate output directory. `SENTINEL_UI_FORCE_EARLY_FAILURE=1` exercises failure reporting before the browser or server starts; the Node regression tests use unique temporary directories and remove only those directories they created.
