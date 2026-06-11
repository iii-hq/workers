/**
 * Resolve the Codex CLI binary for the SDK. The npm-installed SDK locates
 * the vendored CLI relative to its own module path, but the single-file
 * bundle (`deploy: bundle`) cannot carry it — so when the operator has not
 * pinned `codex_executable` in config.yaml, fall back to the `codex` binary
 * on PATH.
 */

import { accessSync, constants } from 'node:fs';
import { delimiter, join } from 'node:path';

export function resolveCodexExecutable(configured: string): string {
  if (configured) return configured;
  const path = process.env.PATH ?? '';
  for (const dir of path.split(delimiter)) {
    if (!dir) continue;
    const candidate = join(dir, 'codex');
    try {
      accessSync(candidate, constants.X_OK);
      return candidate;
    } catch {
      // keep scanning
    }
  }
  return '';
}
