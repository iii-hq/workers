# ide — evidence

## Frontend, layer 1 (static)
- `pnpm build` in ide/ui: tsc clean, lint 0 errors / 0 warnings, dist/page.js sha256 9ae766fbad95bde28ac2c76cda1370df53561aafa84760418849f678da59c481.
- `pnpm test`: 44 files, 363 tests passed, including the new `scm-view.test.tsx` case (folder `a` actions receive `a/b/y.txt,a/x.txt`; `a/b` receives `a/b/y.txt`; Timeline markup unchanged by existing tests).

## Frontend, layers 2-4 — blocked
- `console::ui-manifest` serves `ide/page.js` hash 020705eef9f0b844 (old build).
- The live ide worker is PID 2217192, started by compose project `my-project` as `cargo run --locked --bin ide` in /home/layon/workspaces/worktrees/workers-main-20260919/ide. UI is embedded at compile time, so changes in /home/layon/workspaces/workers/ide are not visible in the ADE until that worker runs this checkout.

## Delta: images open directly (C5)
- `opensFileDirectly` in scm-view.ts (existing, non-deleted raster image per `imageMimeFromPath`; SVG excluded); SourceControlTab row click calls `onOpenFile` for those, diff otherwise.
- `pnpm build`: tsc clean, lint 0/0. `pnpm test`: 44 files, 364 tests passed (new case covers png, JPEG untracked, deleted png, svg, txt).

## Criteria
- C1-C5: not observed yet (blocked on delivery).
