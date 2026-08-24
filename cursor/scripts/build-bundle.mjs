#!/usr/bin/env node

import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const packageJsonRequirePattern =
  /createRequire\(\s*import\.meta\.url\s*\)\s*\(\s*"\.\.\/package\.json"\s*\)/;

const inlinePackageJson = {
  name: 'iii-inline-sdk-package-json',
  setup(builder) {
    builder.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
      const [source, pkg] = await Promise.all([
        readFile(args.path, 'utf8'),
        readFile(join(root, 'node_modules/iii-sdk/package.json'), 'utf8'),
      ]);
      const { version } = JSON.parse(pkg);
      if (!packageJsonRequirePattern.test(source)) {
        throw new Error('iii-sdk package.json lookup pattern was not found during bundling');
      }
      return {
        contents: source.replace(
          new RegExp(packageJsonRequirePattern.source, 'g'),
          JSON.stringify({ version }),
        ),
        loader: 'js',
      };
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
