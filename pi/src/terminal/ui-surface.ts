/**
 * The console page's surface on the bus: one content function serving both
 * assets, and one `console::ui::asset` observer per asset (args
 * `{ kind, path }`, target the content function). The console fetches each
 * asset by calling through its observer with `{ path }`.
 */

import type { IIIClient } from 'iii-sdk';

const ASSET_FN = 'console::ui::asset';

export type UiAssets = { page_js: string; styles_css: string };

export function registerUiSurface(iii: IIIClient, worker: string, assets: UiAssets): void {
  const files: Record<string, { content: string; content_type: string }> = {
    [`${worker}/page.js`]: { content: assets.page_js, content_type: 'text/javascript' },
    [`${worker}/styles.css`]: { content: assets.styles_css, content_type: 'text/css' },
  };

  iii.registerFunction(
    `${worker}::ui-content`,
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

  const observed = [
    { kind: 'script', path: `${worker}/page.js` },
    { kind: 'style', path: `${worker}/styles.css` },
  ];
  for (const args of observed) {
    const handle = iii.observe({ function_id: ASSET_FN, args }, `${worker}::ui-content`);
    handle.onStateChange((state, reason) => {
      if (state !== 'error') return;
      console.warn(
        `console ui asset ${args.path} rejected: ${reason?.code ?? 'OBSERVER_ERROR'} ${reason?.message ?? ''}`,
      );
    });
  }
}
