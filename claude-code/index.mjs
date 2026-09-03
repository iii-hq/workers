/**
 * Source-checkout bootstrap for manual `node ./index.mjs` use.
 *
 * A published worker starts `dist/bundle/index.mjs` directly because the
 * release archive preserves that path. From a source checkout this bootstrap
 * builds the worker if needed and then starts it.
 *
 * TODO(iii-mono): remove the build step below once `iii compose` honours a
 * manifest's `scripts.install` as a container's default `pre_run`. The build
 * belongs in an install hook, not in the start path — see
 * `crates/iii-compose/src/manifest.rs`, which reads only `scripts.start`.
 */

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const entry = join(root, 'dist', 'index.js');

// Only when there is nothing to run. A stale `dist` is the business of
// whoever edited the source: rebuilding on every boot would add a minute to
// each restart of an already-built worker.
if (!existsSync(entry)) {
  for (const args of [
    ['install', '--frozen-lockfile'],
    ['run', 'build'],
  ]) {
    const step = spawnSync('pnpm', args, { cwd: root, stdio: 'inherit' });
    if (step.error || step.status !== 0) {
      console.error(
        `claude-code: \`pnpm ${args.join(' ')}\` failed (${step.error?.message ?? `exit ${step.status}`}); build it by hand and start again`,
      );
      process.exit(step.status ?? 1);
    }
  }
}

await import(entry);
