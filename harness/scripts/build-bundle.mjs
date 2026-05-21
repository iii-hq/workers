#!/usr/bin/env node

/**
 * Single-file ESM bundle for harness-node (`dist/bundle/index.mjs`).
 *
 * Two real-world bundling quirks the CLI form of esbuild can't handle:
 *
 *   1. iii-sdk reads its own version at module-init via
 *        `const { version } = createRequire(import.meta.url)("../package.json");`
 *      That `createRequire` call survives bundling and, at runtime, resolves
 *      `../package.json` relative to OUR bundle path — which has no
 *      sibling `package.json`. The `inlinePackageJson` plugin below rewrites
 *      that exact call to a literal object before bundling.
 *
 *   2. `pino` configures a `pino/file` transport in non-production mode that
 *      reaches for `__dirname` (undefined in ESM) when bundled. Forcing
 *      `process.env.NODE_ENV` to `"production"` at build time picks the
 *      no-transport branch in `runtime/otel.ts` and avoids the issue.
 *
 * Also injects a `createRequire` banner so CJS deps (commander, pino) that
 * call `require("node:events")` etc. continue to work inside the ESM bundle.
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
