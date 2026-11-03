import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { registerUiSurface } from '../../src/terminal/ui-surface.js';
import { fakeIii } from '../_helpers/fake-iii.js';

const ASSETS = { page_js: 'export default function Page() {}', styles_css: '.pi {}' };

describe('console ui surface', () => {
  it('registerUi registers the surface under this worker with the compiled assets', () => {
    const ui = readFileSync(new URL('../../src/terminal/ui.ts', import.meta.url), 'utf8');
    expect(ui).toContain("const WORKER = 'pi';");
    expect(ui).toContain(
      'registerUiSurface(iii, WORKER, { page_js: PAGE_JS, styles_css: STYLES_CSS });',
    );
  });

  it('observes console::ui::asset once per asset, args { kind, path }, target pi::ui-content', () => {
    const fake = fakeIii();
    registerUiSurface(fake.iii, 'pi', ASSETS);
    expect(fake.observed).toEqual([
      {
        source: { function_id: 'console::ui::asset', args: { kind: 'script', path: 'pi/page.js' } },
        target: 'pi::ui-content',
      },
      {
        source: {
          function_id: 'console::ui::asset',
          args: { kind: 'style', path: 'pi/styles.css' },
        },
        target: 'pi::ui-content',
      },
    ]);
  });

  it('serves each observed path from pi::ui-content, which is internal', async () => {
    const fake = fakeIii();
    registerUiSurface(fake.iii, 'pi', ASSETS);
    expect(fake.functions.get('pi::ui-content')?.options).toMatchObject({
      metadata: { internal: true, trace_hidden: true },
      request_format: { required: ['path'] },
    });
    const content = fake.registered.get('pi::ui-content');
    await expect(content?.({ path: 'pi/page.js' })).resolves.toEqual({
      content: ASSETS.page_js,
      content_type: 'text/javascript',
    });
    await expect(content?.({ path: 'pi/styles.css' })).resolves.toEqual({
      content: ASSETS.styles_css,
      content_type: 'text/css',
    });
    await expect(content?.({ path: 'pi/other.js' })).rejects.toThrow(
      'unknown ui asset: pi/other.js',
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
    registerUiSurface(fake.iii, 'pi', ASSETS);
    expect(listeners).toHaveLength(2);
    listeners[0]('active');
    expect(warn).not.toHaveBeenCalled();
    listeners[1]('error', { code: 'UI_ASSET_SUPERSEDED', message: 'newer observer' });
    expect(warn).toHaveBeenCalledWith(
      'console ui asset pi/styles.css rejected: UI_ASSET_SUPERSEDED newer observer',
    );
    warn.mockRestore();
  });
});
