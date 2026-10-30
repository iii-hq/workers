import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { registerUiSurface } from '../../src/terminal/ui-surface.js';
import { fakeIii } from '../_helpers/fake-iii.js';

const ASSETS = { page_js: 'export default function Page() {}', styles_css: '.claude {}' };

describe('console ui surface', () => {
  it('registerUi registers the surface under this worker with the compiled assets', () => {
    const ui = readFileSync(new URL('../../src/terminal/ui.ts', import.meta.url), 'utf8');
    expect(ui).toContain("const WORKER = 'claude';");
    expect(ui).toContain(
      'registerUiSurface(iii, WORKER, { page_js: PAGE_JS, styles_css: STYLES_CSS });',
    );
  });

  it('observes console::ui::asset once per asset, args { kind, path }, target claude::ui-content', () => {
    const fake = fakeIii();
    registerUiSurface(fake.iii, 'claude', ASSETS);
    expect(fake.observed).toEqual([
      {
        source: {
          function_id: 'console::ui::asset',
          args: { kind: 'script', path: 'claude/page.js' },
        },
        target: 'claude::ui-content',
      },
      {
        source: {
          function_id: 'console::ui::asset',
          args: { kind: 'style', path: 'claude/styles.css' },
        },
        target: 'claude::ui-content',
      },
    ]);
  });

  it('serves each observed path from claude::ui-content, which is internal', async () => {
    const fake = fakeIii();
    registerUiSurface(fake.iii, 'claude', ASSETS);
    expect(fake.functions.get('claude::ui-content')?.options).toMatchObject({
      metadata: { internal: true, trace_hidden: true },
      request_format: { required: ['path'] },
    });
    const content = fake.registered.get('claude::ui-content');
    await expect(content?.({ path: 'claude/page.js' })).resolves.toEqual({
      content: ASSETS.page_js,
      content_type: 'text/javascript',
    });
    await expect(content?.({ path: 'claude/styles.css' })).resolves.toEqual({
      content: ASSETS.styles_css,
      content_type: 'text/css',
    });
    await expect(content?.({ path: 'claude/other.js' })).rejects.toThrow(
      'unknown ui asset: claude/other.js',
    );
  });

  it('logs an asset the console rejects', () => {
    const fake = fakeIii();
    type Listener = (state: string, reason?: { code?: string; message?: string }) => void;
    const listeners: Listener[] = [];
    (fake.iii as { observe: unknown }).observe = () => ({
      onStateChange: (listener: Listener) => {
        listeners.push(listener);
        return () => {};
      },
    });
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    registerUiSurface(fake.iii, 'claude', ASSETS);
    expect(listeners).toHaveLength(2);
    listeners[0]('active');
    expect(warn).not.toHaveBeenCalled();
    listeners[1]('error', { code: 'UI_ASSET_SUPERSEDED', message: 'newer observer' });
    expect(warn).toHaveBeenCalledWith(
      'console ui asset claude/styles.css rejected: UI_ASSET_SUPERSEDED newer observer',
    );
    warn.mockRestore();
  });
});
