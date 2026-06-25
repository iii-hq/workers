/**
 * Resolve the OpenCode CLI binary. The single-file bundle (`deploy: bundle`)
 * does not carry the `opencode` CLI, so when the operator has not pinned
 * `opencode_executable` in config.yaml, fall back to the `opencode` binary on
 * PATH.
 */

import { accessSync, constants } from 'node:fs';
import { delimiter, join } from 'node:path';

export function resolveOpencodeExecutable(configured: string): string {
  if (configured) return configured;
  const path = process.env.PATH ?? '';
  for (const dir of path.split(delimiter)) {
    if (!dir) continue;
    const candidate = join(dir, 'opencode');
    try {
      accessSync(candidate, constants.X_OK);
      return candidate;
    } catch {
      // keep scanning
    }
  }
  return 'opencode';
}
