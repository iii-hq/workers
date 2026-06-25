#!/usr/bin/env node

/**
 * Single-file ESM bundle for opencode (`dist/bundle/index.mjs`).
 *
 * Mirrors harness/scripts/build-bundle.mjs:
 *
 *   1. iii-sdk reads its own version at module-init via
 *        `createRequire(import.meta.url)("../package.json")`
 *      which resolves relative to the bundle path at runtime. The
 *      `inlinePackageJson` plugin rewrites that call to a literal object.
 *
 *   2. The worker spawns the `opencode` CLI as a subprocess (no vendored
 *      SDK in the bundle); the `opencode_executable` config option, or an
 *      `opencode` binary on PATH, resolves it at runtime.
 */

import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const root = join(__dirname, '..');

/** @type {import('esbuild').Plugin} */
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
  entryPoints: [join(root, 'src/index.ts')],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: join(root, 'dist/bundle/index.mjs'),
  legalComments: 'none',
  external: ['fsevents'],
  banner: {
    js: "import{createRequire as __iiiCR}from'module';const require=__iiiCR(import.meta.url);",
  },
  define: {
    'process.env.NODE_ENV': '"production"',
  },
  plugins: [inlinePackageJson],
  logLevel: 'info',
});
