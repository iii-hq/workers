/**
 * Resolve the Claude Code CLI binary for the Agent SDK. The npm-installed
 * SDK ships a native CLI as an optional platform dependency, but the
 * single-file bundle (`deploy: bundle`) cannot carry it — so when the
 * operator has not pinned `claude_executable` in config.yaml, fall back to
 * the `claude` binary on PATH.
 */

import { accessSync, constants } from 'node:fs';
import { delimiter, join } from 'node:path';

export function resolveClaudeExecutable(configured: string): string {
  if (configured) return configured;
  const path = process.env.PATH ?? '';
  for (const dir of path.split(delimiter)) {
    if (!dir) continue;
    const candidate = join(dir, 'claude');
    try {
      accessSync(candidate, constants.X_OK);
      return candidate;
    } catch {
      // keep scanning
    }
  }
  return '';
}
