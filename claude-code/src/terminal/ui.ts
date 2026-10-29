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

const WORKER = 'claude';
const ASSET_FN = 'console::ui::asset';

export function registerUi(iii: IIIClient): void {
  const files: Record<string, { content: string; content_type: string }> = {
    [`${WORKER}/page.js`]: { content: PAGE_JS, content_type: 'text/javascript' },
    [`${WORKER}/styles.css`]: { content: STYLES_CSS, content_type: 'text/css' },
  };

  iii.registerFunction(
    `${WORKER}::ui-content`,
    async ({ path }: { path: string }) => {
      const file = files[path];
      if (!file) throw new Error(`unknown ui asset: ${path}`);
      return file;
    },
    {
      description: 'Console UI asset content (console plumbing).',
      request_format: {
        type: 'object',
        required: ['path'],
        properties: { path: { type: 'string' } },
      },
      response_format: {
        type: 'object',
        required: ['content'],
        properties: { content: { type: 'string' }, content_type: { type: 'string' } },
      },
      metadata: { internal: true, trace_hidden: true },
    },
  );

  const assets = [
    { kind: 'script', path: `${WORKER}/page.js` },
    { kind: 'style', path: `${WORKER}/styles.css` },
  ];
  for (const args of assets) {
    const handle = iii.observe({ function_id: ASSET_FN, args }, `${WORKER}::ui-content`);
    handle.onStateChange((state, reason) => {
      if (state !== 'error') return;
      console.warn(
        `console ui asset ${args.path} rejected: ${reason?.code ?? 'OBSERVER_ERROR'} ${reason?.message ?? ''}`,
      );
    });
  }
}
