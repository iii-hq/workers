/**
 * The injectable console page (docs/sops/injectable-console-ui.md): one
 * content function serving both assets, and one `console::ui::asset` observer
 * per asset (args `{ kind, path }`, target the content function). The console
 * fetches each asset by calling through its observer with `{ path }`. An
 * observer belongs to this worker's connection and the SDK replays it on
 * reconnect, so the page lives and dies with this worker — which is the design.
 *
 * The assets are compiled in (`src/ui-assets.generated.ts`, written by
 * `ui/build.mjs`), because `deploy: bundle` ships ONE file and a runtime read
 * of `ui/dist` would serve nothing once published — the Node equivalent of
 * the Rust workers' `include_str!`. `npm run build` regenerates it; it is not
 * committed.
 */

import type { IIIClient } from 'iii-sdk';
import { PAGE_JS, STYLES_CSS } from '../ui-assets.generated.js';
import { registerUiSurface } from './ui-surface.js';

const WORKER = 'pi';

export function registerUi(iii: IIIClient): void {
  registerUiSurface(iii, WORKER, { page_js: PAGE_JS, styles_css: STYLES_CSS });
}
