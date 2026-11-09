#!/usr/bin/env node
/**
 * Build (or check) the iii-integration plugin.
 *
 *   node scripts/build-iii-integration.mjs           build
 *   node scripts/build-iii-integration.mjs --check   verify the committed build
 *
 * Build:
 *   1. Bundle the MCP server into one minified file inside the plugin
 *      (`iii-integration/server/index.mjs`). It is COMMITTED: a plugin installed
 *      from git (a Claude Code marketplace) is a copy of the folder, with no
 *      build step of its own.
 *   2. Write `iii-integration/server/BUILD.json`: a hash of every input the
 *      bundle was made from (each project source file esbuild read, the
 *      lockfile, this script) and of the output.
 *   3. Embed every plugin file into `src/iii-integration/assets.generated.ts`
 *      (git-ignored). `deploy: bundle` ships only `dist/bundle/index.mjs`, so
 *      the worker carries the plugin inside itself; CI's deploy build runs
 *      this step through `build:bundle`, like `ui/build.mjs`.
 *
 * Check (CI, `check:committed-build`): recompute the input and output hashes
 * from the stamp and fail when they differ, i.e. a source changed without a
 * rebuild, or the bundle was edited by hand. It does not rebuild: comparing
 * rebuilt bytes would also fail whenever an install resolved a dependency
 * differently from the lockfile, which is not what the check is about.
 *
 * Minified because the terminal half writes the server through
 * `shell::fs::write`, which hosts commonly cap at 1 MiB per file.
 */

import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const pluginRoot = join(root, 'iii-integration');
const OUTPUT = 'iii-integration/server/index.mjs';
const STAMP = 'iii-integration/server/BUILD.json';
const REBUILD = 'run `pnpm run build:iii-integration` in claude-code/ and commit the result';

/** The files that make up the plugin, relative to its root. */
const FILES = ['.claude-plugin/plugin.json', '.mcp.json', 'README.md', 'server/index.mjs'];

/** Inputs that shape the bundle besides the sources esbuild reports. */
const EXTRA_INPUTS = ['pnpm-lock.yaml', 'scripts/build-iii-integration.mjs'];

const posix = (p) => p.split(sep).join('/');

async function sha256(path) {
  return createHash('sha256')
    .update(await readFile(join(root, path)))
    .digest('hex');
}

async function inputsHash(inputs) {
  const hash = createHash('sha256');
  for (const path of inputs)
    hash
      .update(path)
      .update('\0')
      .update(await sha256(path))
      .update('\0');
  return hash.digest('hex');
}

async function check() {
  let stamp;
  try {
    stamp = JSON.parse(await readFile(join(root, STAMP), 'utf8'));
  } catch {
    throw new Error(`${STAMP} is missing or unreadable; ${REBUILD}`);
  }
  const missing = [];
  for (const path of stamp.inputs ?? []) {
    await readFile(join(root, path)).catch(() => missing.push(path));
  }
  if (missing.length)
    throw new Error(`build inputs were removed (${missing.join(', ')}); ${REBUILD}`);
  if ((await inputsHash(stamp.inputs)) !== stamp.inputs_sha256) {
    throw new Error(`${OUTPUT} is out of date with its sources; ${REBUILD}`);
  }
  if ((await sha256(OUTPUT).catch(() => '')) !== stamp.output_sha256) {
    throw new Error(
      `${OUTPUT} does not match the build recorded in ${STAMP} (edited by hand?); ${REBUILD}`,
    );
  }
  console.log(`${OUTPUT} is up to date (${stamp.inputs.length} inputs)`);
}

async function build() {
  const { build: esbuild } = await import('esbuild');

  /** Same rewrite as build-bundle.mjs: iii-sdk reads ../package.json at init. */
  const inlinePackageJson = {
    name: 'iii-inline-sdk-package-json',
    setup(b) {
      b.onLoad({ filter: /iii-sdk[\\/]dist[\\/]index\.mjs$/ }, async (args) => {
        const source = await readFile(args.path, 'utf8');
        const pkg = await readFile(join(dirname(args.path), '..', 'package.json'), 'utf8');
        const { version } = JSON.parse(pkg);
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

  const result = await esbuild({
    entryPoints: [join(root, 'src/iii-integration/main.ts')],
    absWorkingDir: root,
    bundle: true,
    minify: true,
    metafile: true,
    platform: 'node',
    target: 'node22',
    format: 'esm',
    outfile: join(root, OUTPUT),
    legalComments: 'none',
    external: ['fsevents'],
    banner: {
      js: "import{createRequire as __iiiCR}from'module';const require=__iiiCR(import.meta.url);",
    },
    define: { 'process.env.NODE_ENV': '"production"' },
    plugins: [inlinePackageJson],
    logLevel: 'info',
  });

  // Project sources only: dependencies are pinned by the lockfile instead.
  const sources = Object.keys(result.metafile.inputs)
    .map((p) => posix(relative(root, join(root, p))))
    .filter((p) => !p.includes('node_modules/') && !p.startsWith('../'));
  const inputs = [...new Set([...sources, ...EXTRA_INPUTS])].sort();
  const stamp = {
    note: `Generated by scripts/build-iii-integration.mjs; CI fails when it is stale. To refresh: ${REBUILD}.`,
    inputs,
    inputs_sha256: await inputsHash(inputs),
    output_sha256: await sha256(OUTPUT),
  };
  await writeFile(join(root, STAMP), `${JSON.stringify(stamp, null, 2)}\n`);

  const files = [];
  for (const path of FILES) {
    files.push({ path, content: await readFile(join(pluginRoot, path), 'utf8') });
  }
  await writeFile(
    join(root, 'src/iii-integration/assets.generated.ts'),
    `// Generated by scripts/build-iii-integration.mjs — do not edit.
/* biome-ignore-all lint: generated file */
export const INTEGRATION_FILES: { path: string; content: string }[] = ${JSON.stringify(files)};
`,
  );
}

try {
  await (process.argv.includes('--check') ? check() : build());
} catch (err) {
  console.error(`iii-integration: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
}
