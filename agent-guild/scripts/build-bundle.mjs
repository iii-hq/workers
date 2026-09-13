import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
await build({
  entryPoints: [join(root, 'src/index.ts')],
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  outfile: join(root, 'dist/bundle/index.mjs'),
  legalComments: 'eof',
  banner: {
    js: "import{createRequire as __iiiCR}from'node:module';const require=__iiiCR(import.meta.url);",
  },
  plugins: [
    {
      name: 'iii-sdk-package-version',
      setup(builder) {
        builder.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
          const source = await readFile(args.path, 'utf8');
          const { version } = JSON.parse(
            await readFile(join(root, 'node_modules/iii-sdk/package.json'), 'utf8'),
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
    },
  ],
});
