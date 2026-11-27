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
  'ui/ConfirmDialog': 'ConfirmDialog',
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
  function setup(
    t,
    {
      heartbeat = true,
      plainHttp = false,
      missingForkDirectory = false,
      refuseConfigWatch = false,
    } = {},
  ) {
    const url = new URL('http://127.0.0.1/');
    if (plainHttp) url.searchParams.set('no-random-uuid', '');
    if (missingForkDirectory) url.searchParams.set('missing-fork-directory', '');
    if (refuseConfigWatch) url.searchParams.set('refuse-config-watch', '');
    const { window } = new JSDOM('<div id="root"></div>', {
      url: url.href,
      pretendToBeVisual: true,
      runScripts: 'outside-only',
    });
    const { document } = window;
    t.after(() => window.close());
    const interval = window.setInterval.bind(window);
    if (heartbeat) {
      window.setInterval = (callback, delay, ...args) =>
        interval(callback, Math.min(delay, 50), ...args);
    }
    window.eval(code);
    const until = (read) =>
      new Promise((resolve, reject) => {
        const observer = new window.MutationObserver(check);
        const timer = setTimeout(() => {
          observer.disconnect();
          document.removeEventListener('focusin', check);
          reject(new Error(`Expected UI state was not rendered: ${document.body.textContent}`));
        }, 2000);
        function check() {
          const value = read();
          if (!value) return;
          clearTimeout(timer);
          observer.disconnect();
          document.removeEventListener('focusin', check);
          resolve(value);
        }
        observer.observe(document.body, {
          childList: true,
          subtree: true,
          characterData: true,
          attributes: true,
        });
        document.addEventListener('focusin', check);
        check();
      });
    const click = async (name) => {
      const button = await until(() =>
        Array.from(document.querySelectorAll('button')).find(
          (el) =>
            (el.getAttribute('aria-label') ?? el.textContent) === name &&
            el.getAttribute('aria-disabled') !== 'true',
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
    const focus = (name) =>
      until(() => {
        const element = document.activeElement;
        return (
          element?.matches('button') &&
          (element.getAttribute('aria-label') ?? element.textContent) === name
        );
      });
    return { document, click, text, shortcut, follow, focus, until };
  }

  for (const refuseConfigWatch of [false, true]) {
    await test(`distinguishes partial, unavailable and empty inventory with ${refuseConfigWatch ? 'unconfirmed' : 'confirmed'} access`, async (t) => {
      const { click, document, text, shortcut, until } = setup(t, { refuseConfigWatch });
      const choose = (value) => {
        const select = document.querySelector('select');
        select.value = value;
        select.dispatchEvent(new document.defaultView.Event('change', { bubbles: true }));
      };
      await text('Console original');
      choose('empty');
      await text('No sessions listed');
      assert.doesNotMatch(document.body.textContent, /Host sessions unavailable/);
      choose('unavailable');
      await text('Host sessions unavailable');
      assert.doesNotMatch(document.body.textContent, /No sessions listed|private-inventory/);
      choose('partial');
      await text('Worker run');
      assert.match(document.body.textContent, /Host sessions unavailable/);
      shortcut('KeyR');
      await click('Refresh sessions');
      await until(() => document.querySelector('[aria-label="Message Worker run"]'));
      assert.match(document.body.textContent, /Host sessions unavailable/);
      assert.doesNotMatch(
        document.body.textContent,
        /No sessions listed|private-inventory|external-only/,
      );
      choose('empty');
      await text('No sessions listed');
      assert.doesNotMatch(document.body.textContent, /Host sessions unavailable|private-inventory/);
    });
  }

  await test('hides replies on the config event without waiting for refresh or polling', async (t) => {
    const { follow, shortcut, text, document } = setup(t, { heartbeat: false });
    await follow();
    shortcut('KeyC');
    shortcut('KeyU');
    await text('Review update 2:');
    shortcut('KeyR');
    await text('Replies unavailable');
    assert.equal(document.querySelector('[role="log"]'), null);
    assert.doesNotMatch(document.body.textContent, /Review update 2:|Design review|external-only/);
    assert.equal(document.documentElement.dataset.replyWatches, '0');
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('does not render an abandoned reply when the active branch is empty', async (t) => {
    const { follow, shortcut, text, document } = setup(t);
    await follow();
    shortcut('KeyS');
    await text('No assistant reply yet.');
    assert.equal(document.querySelector('[role="log"]').textContent, 'No assistant reply yet.');
    assert.match(document.querySelector('main').textContent, /Terminals created:\s*0$/);
  });

  await test('does not publish an abandoned delivery when another branch has a reply', async (t) => {
    const { follow, shortcut, text, document } = setup(t);
    await follow();
    shortcut('KeyN');
    await text('Current branch answer');
    assert.equal(document.querySelector('[role="log"]').textContent, 'Current branch answer');
  });

  await test('recovers both replies written before observer admission', async (t) => {
    const { click, follow, shortcut, text, document } = setup(t);
    await click('Delay reply observer admission');
    await follow();
    shortcut('KeyA');
    await text('Review update 3:');
    assert.match(document.querySelector('[role="log"]').textContent, /Review update 2:/);
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
  await test('shows the first snapshot when a reply arrives during its read', async (t) => {
    const { click, shortcut, until, document } = setup(t, { heartbeat: false });
    const log = () =>
      Array.from(document.querySelectorAll('[role="log"] p'), (p) => p.textContent).join('\n');
    const seen = [];
    const record = new document.defaultView.MutationObserver(() => {
      const current = log();
      if (current && seen.at(-1) !== current) seen.push(current);
    });
    record.observe(document.body, { childList: true, subtree: true, characterData: true });
    t.after(() => record.disconnect());
    // The fixture's shortcuts exist only once the preview has rendered.
    await until(() => document.querySelector('[aria-label="Open original: Design review"]'));
    // Hold the first read and the re-read after they read the snapshot.
    shortcut('KeyP');
    await click('Open original: Design review');
    await until(() => document.documentElement.dataset.replyWatches === '1');
    // A wakeup arrives while the first read is still pending.
    shortcut('KeyU');
    shortcut('KeyQ');
    await until(() => log() === 'The layout review is ready.');
    shortcut('KeyQ');
    await until(() => /^Review update 2:/.test(log()));
    assert.deepEqual(seen, [
      'The layout review is ready.',
      'Review update 2: the changes are ready to check.',
    ]);
  });

  await test('follows console, background and finished originals without a terminal', async (t) => {
    const { click, text, document, focus } = setup(t);
    for (const name of ['Console original', 'Background indexing', 'Finished review']) {
      await click(`Open original: ${name}`);
      await text(`Synthetic reply for ${name}.`);
      const viewer = document.querySelector('[role="dialog"]');
      assert.match(viewer.textContent, /Read only.*stay with this conversation/);
      assert.equal(viewer.querySelector('input, textarea, [contenteditable="true"]'), null);
      assert.equal(document.documentElement.dataset.replyWatches, '1');
      await focus('Close');
      await click('Close');
      await focus(`Open original: ${name}`);
      assert.equal(document.documentElement.dataset.replyWatches, '0');
    }
    assert.match(
      document.querySelector('main').textContent,
      /No terminal selected.*Terminal open calls:\s*0.*Terminals created:\s*0/,
    );
  });

  await test('reads the fresh state when an original finished after it was listed', async (t) => {
    const { click, text, document } = setup(t);
    await text('Working · Background');
    await click('Finish background without refreshing');
    assert.match(document.body.textContent, /Working · Background/);
    await click('Open original: Background indexing');
    await text('Background indexing · Done');
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('keeps following a finished background original after it leaves the rail', async (t) => {
    const { click, text, shortcut, document, until } = setup(t);
    const row = '[aria-label="Open original: Background indexing"]';
    await click('Open original: Background indexing');
    await text('Background indexing · Working');
    assert.equal(document.documentElement.dataset.replyWatches, '1');

    shortcut('KeyF');
    await until(() => !document.querySelector(row));
    await text('Background indexing · Done');
    assert.equal(
      document.querySelector('[role="log"]').textContent,
      'Synthetic reply for Background indexing.',
    );
    assert.equal(document.documentElement.dataset.replyWatches, '1');
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('only a confirmed fork creates a copy; reopening still follows the original', async (t) => {
    const { click, follow, shortcut, text, document } = setup(t, {
      plainHttp: true,
      heartbeat: false,
    });
    assert.equal(typeof document.defaultView.crypto.randomUUID, 'undefined');
    const terminal = () => document.querySelector('main').textContent;
    await follow();
    shortcut('KeyU');
    await text('Review update 2:');
    assert.match(terminal(), /Terminal open calls:\s*0/);
    await click('Close');
    await click('Fork Design review');
    await click('Cancel');
    assert.match(terminal(), /Terminal open calls:\s*0/);
    await click('Fork Design review');
    await click('Create fork');
    await text('Separate fork terminal · copy-1');
    await click('Open original: Design review');
    await text('Review update 2:');
    assert.equal(document.documentElement.dataset.replyWatches, '1');
    assert.doesNotMatch(document.querySelector('[role="dialog"]').textContent, /copy-1/);
    assert.match(terminal(), /Terminal open calls:\s*1.*Terminals created:\s*1/);
  });

  await test('dismisses an external fork confirmation as soon as access is revoked', async (t) => {
    const { click, focus, shortcut, text, document, until } = setup(t, { heartbeat: false });
    const original = '[aria-label="Open original: Design review"]';
    const dialog = () => document.querySelector('[role="alertdialog"]');

    await click('Fork Design review');
    await focus('Cancel');
    await text('Fork Design review?');
    assert.ok(dialog());
    shortcut('KeyR');
    await until(() => !document.querySelector(original));
    assert.doesNotMatch(dialog()?.textContent ?? '', /Design review/);
    await until(() => !dialog());
    await focus('Refresh sessions');
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);

    await click('Toggle access');
    await until(() => document.querySelector(original));
    assert.equal(dialog(), null);
    await click('Fork Design review');
    await click('Create fork');
    await text('Separate fork terminal · copy-1');
  });

  await test('clears a fork announcement when access to its original is revoked', async (t) => {
    const { click, focus, shortcut, text, document, until } = setup(t, { heartbeat: false });
    const original = '[aria-label="Open original: Design review"]';
    const status = () => document.querySelector('[role="status"][aria-live="polite"]');

    await click('Fork Design review');
    await focus('Cancel');
    await click('Create fork');
    await text('Separate fork terminal · copy-1');
    await until(() => status()?.textContent === 'Opened a fork of Design review');

    shortcut('KeyR');
    await until(() => !document.querySelector(original));
    assert.equal(status().textContent, '');
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);

    await click('Toggle access');
    await until(() => document.querySelector(original));
    assert.equal(status().textContent, '');
  });

  await test('withholds external-only inventory when the browser config observer is refused', async (t) => {
    const { click, document, shortcut, text } = setup(t, { refuseConfigWatch: true });
    await text('Console original');
    await click('Refresh sessions');
    // Give the mock list and the asynchronous admission decision a render turn.
    await new Promise((resolve) => document.defaultView.requestAnimationFrame(resolve));
    assert.equal(document.documentElement.dataset.configWatch, 'refused');
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);

    shortcut('KeyR');
    await new Promise((resolve) => document.defaultView.requestAnimationFrame(resolve));
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.body.textContent, /Console original/);
  });

  await test('clears cached external inventory and its Fork failure after the browser watch is lost', async (t) => {
    const { click, document, shortcut, text, until } = setup(t, { missingForkDirectory: true });
    await text('Design review');
    await click('Fork Design review');
    await click('Create fork');
    await text('the folder /fixture/external-only is not reachable');

    shortcut('KeyV');
    await until(() => !document.querySelector('[aria-label="Open original: Design review"]'));
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.body.textContent, /Console original/);
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('revalidates cached external rows and Fork notices against the independent worker snapshot', async (t) => {
    const { click, document, shortcut, text, until } = setup(t, { missingForkDirectory: true });
    const original = '[aria-label="Open original: Design review"]';
    const notice = () => document.querySelector('.claude-sessions-notices [role="alert"]');
    await text('/fixture/external-only');
    shortcut('KeyW');
    await until(() => !document.querySelector(original));
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.body.textContent, /Console original/);

    shortcut('KeyW');
    await until(() => document.querySelector(original));
    await click('Fork Design review');
    await click('Create fork');
    await text('the folder /fixture/external-only is not reachable');
    assert.ok(document.querySelector(original));

    shortcut('KeyW');
    await until(() => !document.querySelector(original));
    assert.equal(notice(), null);
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.body.textContent, /Console original/);

    shortcut('KeyW');
    await until(() => document.querySelector(original));
    assert.equal(notice(), null);
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('restores external inventory from a new worker snapshot without a completion event', async (t) => {
    const { click, document, shortcut, text, until } = setup(t);
    const original = '[aria-label="Open original: Design review"]';
    await text('/fixture/external-only');
    shortcut('KeyR');
    await until(() => !document.querySelector(original));
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
    assert.match(document.body.textContent, /Console original/);

    // Only the worker snapshot changes; no config or inventory event is delivered.
    shortcut('KeyW');
    await until(() => document.querySelector(original));
    await click('Open original: Design review');
    await text('The layout review is ready.');
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  for (const order of ['browser first', 'worker first']) {
    await test(`recovers current external inventory when permission confirms ${order}`, async (t) => {
      const { click, document, follow, shortcut, text, until } = setup(t, { heartbeat: false });
      const original = () => document.querySelector('[aria-label="Open original: Design review"]');
      const withheld = async () => {
        await until(() => !original());
        assert.doesNotMatch(document.body.textContent, /Design review|external-only/);
        assert.match(document.body.textContent, /Console original/);
      };
      await text('/fixture/external-only');

      if (order === 'browser first') {
        shortcut('KeyG');
        await withheld();
        await click('Refresh sessions');
        await new Promise((resolve) => document.defaultView.requestAnimationFrame(resolve));
        assert.equal(original(), null);
        shortcut('KeyH');
      } else {
        shortcut('KeyH');
        await text('Waiting for permission confirmation');
        shortcut('KeyJ');
        await withheld();
        await until(() => document.querySelector('[data-inventory-waiting="true"]'));
        assert.equal(original(), null);
        shortcut('KeyK');
      }

      await until(original);
      await click('Refresh sessions');
      await new Promise((resolve) => document.defaultView.requestAnimationFrame(resolve));
      assert.ok(original());
      await follow();
      assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
      shortcut('KeyR');
      await text('Replies unavailable');
      await withheld();
    });
  }

  await test('Escape returns focus to Refresh sessions if access removes the opener', async (t) => {
    const { document, focus, follow, shortcut, text, until } = setup(t, { heartbeat: false });
    await follow();
    await focus('Close');
    shortcut('KeyR');
    await text('Replies unavailable');
    assert.equal(document.querySelector('[aria-label="Open original: Design review"]'), null);
    document.activeElement.dispatchEvent(
      new document.defaultView.KeyboardEvent('keydown', {
        key: 'Escape',
        bubbles: true,
        cancelable: true,
      }),
    );
    await until(() => !document.querySelector('[role="dialog"]'));
    await focus('Refresh sessions');
    assert.equal(document.documentElement.dataset.replyWatches, '0');
  });

  await test('clears an external fork failure on access revocation without affecting console notices', async (t) => {
    const { click, focus, shortcut, document, until } = setup(t, {
      heartbeat: false,
      missingForkDirectory: true,
    });
    const original = '[aria-label="Open original: Design review"]';
    const notice = () => document.querySelector('.claude-sessions-notices [role="alert"]');
    const failFork = async (name, directory) => {
      await click(`Fork ${name}`);
      await focus('Cancel');
      await click('Create fork');
      await until(() => notice()?.textContent.includes(`the folder ${directory} is not reachable`));
    };

    await failFork('Design review', '/fixture/external-only');
    shortcut('KeyR');
    await until(() => !document.querySelector(original));
    assert.equal(notice()?.textContent, undefined);
    assert.doesNotMatch(document.body.textContent, /Design review|external-only/);

    await click('Toggle access');
    await until(() => document.querySelector(original));
    assert.equal(notice()?.textContent, undefined);

    await failFork('Console original', '/fixture/workspace');
    shortcut('KeyR');
    await until(() => !document.querySelector(original));
    assert.match(notice()?.textContent ?? '', /the folder \/fixture\/workspace is not reachable/);
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  await test('a late read from the previous selection cannot replace the current reply', async (t) => {
    const { click, follow, shortcut, text, document } = setup(t, { heartbeat: false });
    await follow();
    shortcut('KeyT');
    await click('Close');
    assert.equal(document.documentElement.dataset.replyWatches, '0');
    await click('Open original: Console original');
    await text('Synthetic reply for Console original.');
    shortcut('KeyY');
    await new Promise((resolve) => document.defaultView.requestAnimationFrame(resolve));
    assert.equal(
      document.querySelector('[role="log"]').textContent,
      'Synthetic reply for Console original.',
    );
    assert.equal(document.documentElement.dataset.replyWatches, '1');
  });

  await test('clears the viewer and reports unavailable when its source is lost', async (t) => {
    const { follow, shortcut, text, document } = setup(t, { heartbeat: false });
    await follow();
    shortcut('KeyL');
    await text('Replies unavailable');
    assert.equal(document.querySelector('[role="log"]'), null);
    assert.doesNotMatch(
      document.body.textContent,
      /The layout review is ready|No assistant reply yet/,
    );
    assert.equal(document.documentElement.dataset.replyWatches, '0');
    assert.match(document.querySelector('main').textContent, /Terminal open calls:\s*0/);
  });

  // Starts a fork of `name` whose describe stays pending until `release` runs.
  async function pendingFork({ click, shortcut, text }, name) {
    // The fixture's shortcuts exist only once the preview has rendered.
    await text(name);
    shortcut('KeyD');
    await click(`Fork ${name}`);
    await click('Create fork');
    await text('Opening…');
  }
  const settle = (document) =>
    new Promise((resolve) => document.defaultView.setTimeout(resolve, 100));
  const openCalls = (document) =>
    document.querySelector('main').textContent.match(/Terminal open calls:\s*(\d+)/)[1];

  await test('a worker-owned row that leaves the list still forks', async (t) => {
    const ui = setup(t, { heartbeat: false });
    const { document, shortcut, text, until } = ui;
    await pendingFork(ui, 'Console original');
    shortcut('KeyI');
    await until(() => !document.querySelector('[aria-label="Open original: Console original"]'));
    shortcut('KeyE');
    await text('Separate fork terminal · copy-1');
    assert.equal(openCalls(document), '1');
  });

  await test('a refresh that keeps the grant and the row still forks', async (t) => {
    const ui = setup(t, { heartbeat: false });
    const { click, document, shortcut, text } = ui;
    await pendingFork(ui, 'Design review');
    await click('Refresh sessions');
    await settle(document);
    shortcut('KeyE');
    await text('Separate fork terminal · copy-1');
    assert.equal(openCalls(document), '1');
  });

  await test('abandons a fork when access is lost while describe is pending', async (t) => {
    const ui = setup(t, { heartbeat: false });
    const { document, shortcut, until } = ui;
    await pendingFork(ui, 'Design review');
    shortcut('KeyR');
    await until(() => !document.querySelector('[aria-label="Open original: Design review"]'));
    shortcut('KeyE');
    await settle(document);
    assert.equal(openCalls(document), '0');
    assert.doesNotMatch(document.body.textContent, /Separate fork terminal|Design review/);
  });

  await test('abandons a fork whose row left the list while describe is pending', async (t) => {
    const ui = setup(t, { heartbeat: false });
    const { document, shortcut, until } = ui;
    await pendingFork(ui, 'Design review');
    shortcut('KeyO');
    await until(() => !document.querySelector('[aria-label="Open original: Design review"]'));
    shortcut('KeyE');
    await settle(document);
    assert.equal(openCalls(document), '0');
    assert.doesNotMatch(document.body.textContent, /Separate fork terminal/);
  });

  await test('a config event during describe abandons the fork', async (t) => {
    const ui = setup(t, { heartbeat: false });
    const { document, shortcut, until } = ui;
    await pendingFork(ui, 'Design review');
    shortcut('KeyM');
    await settle(document);
    // The same grant is confirmed again: the row is listed once more.
    await until(() => document.querySelector('[aria-label="Open original: Design review"]'));
    shortcut('KeyE');
    await settle(document);
    assert.equal(openCalls(document), '0');
    assert.doesNotMatch(document.body.textContent, /Separate fork terminal/);
  });

  const workerRun = async (t) => {
    const ui = setup(t);
    await ui.text('Console original');
    const select = ui.document.querySelector('select');
    select.value = 'partial';
    select.dispatchEvent(new ui.document.defaultView.Event('change', { bubbles: true }));
    return ui;
  };

  await test('headless rows show no Open original control', async (t) => {
    const { document, text, until } = await workerRun(t);
    await text('Worker run');
    await until(() => document.querySelector('[aria-label="Message Worker run"]'));
    assert.equal(document.querySelector('[aria-label="Open original: Worker run"]'), null);
    assert.equal(
      Array.from(document.querySelectorAll('button, [role="button"]')).some(
        (el) => el.textContent === 'Worker run',
      ),
      false,
    );
  });

  await test('headless rows keep Message', async (t) => {
    const { document, text, until } = await workerRun(t);
    await text('Worker run');
    const message = await until(() => document.querySelector('[aria-label="Message Worker run"]'));
    assert.notEqual(message.getAttribute('aria-disabled'), 'true');
  });
}
