// Run from a workspace with the claude-code/ui and ade/web dependencies installed.
// Pass --test to run the synthetic UI regressions without starting the server.
// Build in memory: the HTTP server has no filesystem route, proxy or dev socket.
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { extname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const ade = resolve(root, 'ade/web');
const require = createRequire(resolve(ade, 'package.json'));
const { build } = await import(require.resolve('vite'));
const { default: tailwind } = await import(require.resolve('@tailwindcss/vite'));
const testMode = process.argv.includes('--test');
const portArg = process.argv.indexOf('--port');
const port = portArg < 0 ? 4197 : Number(process.argv[portArg + 1]);
const primitives = {
  'ui/Button': 'Button',
  'ui/ConfirmDialog': 'useConfirm',
  'ui/Dialog': 'Dialog, DialogClose, DialogContent, DialogTitle, DialogDescription',
  'ui/EmptyState': 'EmptyState',
  'ui/IconButton': 'IconButton',
  'ui/List': 'List, ListGroup, ListGroupLabel, ListItem',
  'ui/LiveRegion': 'LiveRegion',
  'ui/Skeleton': 'Skeleton',
  'ui/StatusDot': 'StatusDot',
  'ui/StatusPanel': 'StatusPanel',
  'ui/Toolbar': 'StatusBar',
  'shadcn/button': 'Button as ShadcnButton',
  'shadcn/textarea': 'Textarea as ShadcnTextarea',
};
const { output } = await build({
  root: ade,
  configFile: false,
  publicDir: false,
  resolve: { alias: { '@': resolve(ade, 'src') }, dedupe: ['react', 'react-dom'] },
  plugins: [
    tailwind(),
    {
      name: 'session-preview-console-components',
      enforce: 'pre',
      resolveId: (id) => (id === '@iii-dev/console-ui' ? '\0preview-components' : null),
      load: (id) =>
        id === '\0preview-components'
          ? Object.entries(primitives)
              .map(([file, names]) => `export { ${names} } from '@/components/${file}';`)
              .join('\n')
          : null,
    },
  ],
  build: {
    write: false,
    rollupOptions: {
      input: resolve(import.meta.dirname, 'session-actions-preview.tsx'),
      ...(testMode ? { output: { format: 'iife' } } : {}),
    },
  },
});
const entry = output.find((file) => file.type === 'chunk' && file.isEntry);
if (testMode) {
  await testSessionActions(entry.code);
} else {
  servePreview(entry, output);
}

function servePreview(entry, output) {
  const assets = new Map(
    output.map((file) => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]),
  );
  const css = output
    .filter((file) => file.fileName.endsWith('.css'))
    .map((file) => `<link rel="stylesheet" href="/${file.fileName}">`)
    .join('');
  assets.set(
    '/',
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Claude sessions · synthetic preview</title>${css}<div id="root"></div><script type="module" src="/${entry.fileName}"></script></html>`,
  );
  const types = {
    '.js': 'text/javascript',
    '.css': 'text/css',
    '.woff2': 'font/woff2',
    '.woff': 'font/woff',
  };
  createServer((req, res) => {
    const path = req.url?.split('?')[0];
    const asset = assets.get(path);
    if (!asset || req.method !== 'GET') {
      res.writeHead(404).end();
      return;
    }
    res.writeHead(200, {
      'Content-Type':
        path === '/' ? 'text/html' : types[extname(path)] || 'application/octet-stream',
      'Content-Security-Policy':
        "default-src 'self'; connect-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:",
    });
    res.end(asset);
  }).listen(port, '127.0.0.1', () => console.log(`Synthetic sessions: http://127.0.0.1:${port}/`));
}

// Use the same React entry and mocks as the preview, without starting a server,
// browser, real PTY or engine. The DOM waits for React to publish its result.
async function testSessionActions(code) {
  const { default: assert } = await import('node:assert/strict');
  const { test } = await import('node:test');
  const { JSDOM } = require('jsdom');
  function setup(t) {
    const { window } = new JSDOM('<div id="root"></div>', {
      url: 'http://127.0.0.1/',
      pretendToBeVisual: true,
      runScripts: 'outside-only',
    });
    const { document } = window;
    t.after(() => window.close());
    window.eval(code);
    const until = (read) =>
      new Promise((resolve, reject) => {
        const observer = new window.MutationObserver(check);
        const timer = setTimeout(() => {
          observer.disconnect();
          reject(new Error(`Expected UI state was not rendered: ${document.body.textContent}`));
        }, 2000);
        function check() {
          const value = read();
          if (!value) return;
          clearTimeout(timer);
          observer.disconnect();
          resolve(value);
        }
        observer.observe(document.body, {
          childList: true,
          subtree: true,
          characterData: true,
        });
        check();
      });
    const click = async (name) => {
      const button = await until(() =>
        Array.from(document.querySelectorAll('button')).find(
          (el) => (el.getAttribute('aria-label') ?? el.textContent) === name,
        ),
      );
      button.click();
    };
    const text = (value) => until(() => document.body.textContent.includes(value));
    const shortcut = (code) =>
      window.dispatchEvent(new window.KeyboardEvent('keydown', { altKey: true, code }));
    const follow = async () => {
      await click('Open original: Design review');
      await text('The layout review is ready.');
    };
    return { document, click, text, shortcut, follow };
  }

  await test('does not render an abandoned reply when the active branch is empty', async (t) => {
    const { follow, shortcut, text, document } = setup(t);
    await follow();
    shortcut('KeyS');
    await text('No assistant reply yet.');
    assert.equal(document.querySelector('[role="log"]').textContent, 'No assistant reply yet.');
    assert.match(document.querySelector('main').textContent, /Terminals created:\s*0$/);
  });

  await test('renders every rapid active-branch delivery in order without duplicates', async (t) => {
    const { follow, shortcut, text, document } = setup(t);
    await follow();
    shortcut('KeyB');
    await text('Burst reply 21');
    const displayed = Array.from(document.querySelectorAll('[role="log"] p'), (p) => p.textContent);
    assert.deepEqual(
      displayed,
      Array.from({ length: 21 }, (_, i) => `Burst reply ${i + 1}`),
    );
  });

  await test('one selection resumes a finished original after its detached PTY expires', async (t) => {
    const { click, text, document } = setup(t);
    await click('Open original: Finished review');
    await text('Finished original at workspace trust');
    await click('Open original: Console original');
    await text('Original console PTY');
    await click('Open original: Finished review');
    await text('Finished resume on reopen: reused');
    assert.match(document.querySelector('main').textContent, /Terminals created:\s*1/);
    await click('Open original: Console original');
    await text('Original console PTY');
    await click('Expire detached finished PTY');
    await click('Open original: Finished review');
    await text('Finished resume on reopen: replaced');
    assert.match(document.querySelector('main').textContent, /Terminals created:\s*2/);
    assert.doesNotMatch(document.body.textContent, /That did not go through/);
  });
}
