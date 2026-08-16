#!/usr/bin/env node

/**
 * Single-file ESM bundle (`dist/bundle/index.mjs`) for registry publish.
 * typescript stays external so the uncompressed bundle stays under iii's cap.
 */

import { copyFile, readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const root = join(__dirname, '..');

/** @type {import("esbuild").Plugin} */
const inlinePackageJson = {
  name: 'iii-inline-sdk-package-json',
  setup(b) {
    b.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
      const [source, pkg] = await Promise.all([
        readFile(args.path, 'utf8'),
        readFile(join(root, 'node_modules/iii-sdk/package.json'), 'utf8'),
      ]);
      const { version } = JSON.parse(pkg);
      const replaced = source.replace(
        /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*"\.\.\/package\.json"\s*\)/g,
        JSON.stringify({ version }),
      );
      return { contents: replaced, loader: 'js' };
    });
  },
};

await build({
  entryPoints: [join(root, 'src/index.js')],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: join(root, 'dist/bundle/index.mjs'),
  legalComments: 'none',
  external: ['typescript', 'fsevents'],
  banner: {
    js: "import{createRequire as __iiiCR}from'module';const require=__iiiCR(import.meta.url);",
  },
  define: {
    'process.env.NODE_ENV': '"production"',
  },
  plugins: [inlinePackageJson],
  logLevel: 'info',
});

// dist/ is bind-mounted empty inside libkrun. Copy next to iii.worker.yaml so
// `iii worker add` can run `node ./index.mjs` after `cp sandbox.mjs index.mjs`.
await copyFile(join(root, 'dist/bundle/index.mjs'), join(root, 'sandbox.mjs'));
