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

`sp-selector-data.ts` is a synthetic-only catalog: one unbroken 3500-character description, long names/IDs, multiple scrollable entries, Default, and an invalid-inheritance entry. `sp-selector-after.mjs` verifies actual listbox and option bounds, complete text, normal pointer selection, disabled entries, native keyboard/focus and cancel at 1440×1000, 390×844, and a 520px pane in a 1440×1000 viewport, dark/light with reduced motion. `smoke.mjs` covers preserved investigation behavior. No image bytes pass through model context. The Escape/focus resume writes new `sr-after*` and `sr-regression*` artifacts, preserving prior `sp-selector-*` captures. Scripts write these artifacts under `sentinel/docs/evidence/group-investigation-a73c/`.

## Before provenance

The preserved `sp-selector-before.json` and six `sp-selector-before-*.png` were captured before this selector patch, from the real shared Select through the same fixture and synthetic catalog. Baseline job43 completed successfully; desktop/pane option list width was 23333.78px and normal Default selection failed; phone option width was 24466.47px. `sp-selector-before.json` records source/CSS/catalog hashes, scenario conditions and observations. These are not fabricated baseline images and must not be overwritten by the corrected UI.

`sp-selector-baseline.mjs` documents/reproduces the historical measurement procedure against the pre-patch source. Do not run it against corrected source and call that a before capture. Use the preserved artifacts and hashes or an authorized isolated checkout of the exact recorded pre-patch tree. No historical artifacts are deleted. Screenshots are captured mechanically; visual pixel review must be reported separately by a reviewer who actually views them.
