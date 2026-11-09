#!/usr/bin/env node
/**
 * Bundle the iii-directory plugin's MCP server into one file inside the plugin
 * (`directory-plugin/server/index.mjs`), so the directory is self-contained:
 * `claude --plugin-dir ./directory-plugin` needs nothing but Node 22.
 */

import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/** Same rewrite as build-bundle.mjs: iii-sdk reads ../package.json at init. */
const inlinePackageJson = {
  name: 'iii-inline-sdk-package-json',
  setup(b) {
    b.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
      const source = await readFile(args.path, 'utf8');
      const { version } = JSON.parse(
        await readFile(join(dirname(args.path), '..', 'package.json'), 'utf8'),
      );
      return {
        contents: source.replace(
          /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*"\.\.\/package\.json"\s*\)/g,
          JSON.stringify({ version }),
        ),
        loader: 'js',
      };
    });
  },
};

await build({
  entryPoints: [join(root, 'src/directory-plugin/main.ts')],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: join(root, 'directory-plugin/server/index.mjs'),
  legalComments: 'none',
  external: ['fsevents'],
  banner: {
    js: "import{createRequire as __iiiCR}from'module';const require=__iiiCR(import.meta.url);",
  },
  define: { 'process.env.NODE_ENV': '"production"' },
  plugins: [inlinePackageJson],
  logLevel: 'info',
});
