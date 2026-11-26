// Run from a workspace with the claude-code/ui and ade/web dependencies installed.
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
    rollupOptions: { input: resolve(import.meta.dirname, 'session-actions-preview.tsx') },
  },
});
const assets = new Map(
  output.map((file) => [`/${file.fileName}`, file.type === 'asset' ? file.source : file.code]),
);
const entry = output.find((file) => file.type === 'chunk' && file.isEntry);
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
    'Content-Type': path === '/' ? 'text/html' : types[extname(path)] || 'application/octet-stream',
    'Content-Security-Policy':
      "default-src 'self'; connect-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:",
  });
  res.end(asset);
}).listen(port, '127.0.0.1', () => console.log(`Synthetic sessions: http://127.0.0.1:${port}/`));
