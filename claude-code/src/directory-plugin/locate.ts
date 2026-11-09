/**
 * Where the shipped iii-directory plugin lives on this worker's disk.
 *
 * The same directory a person passes to `claude --plugin-dir`; the worker hands
 * it to headless turns. It sits at the package root, so the path differs by one
 * level between `tsx src/` / `dist/` and `dist/bundle/`. Only a built plugin
 * (one with its server bundle) counts.
 */

import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function directoryPluginDir(from: string = import.meta.url): {
  dir: string;
  detail: string;
} {
  const here = dirname(fileURLToPath(from));
  const candidates = [
    process.env.III_DIRECTORY_PLUGIN_DIR ?? '',
    resolve(here, '..', '..', 'directory-plugin'),
    resolve(here, '..', 'directory-plugin'),
    resolve(here, '..', '..', '..', 'directory-plugin'),
  ].filter(Boolean);
  for (const dir of candidates) {
    if (
      existsSync(join(dir, '.claude-plugin', 'plugin.json')) &&
      existsSync(join(dir, 'server', 'index.mjs'))
    ) {
      return { dir, detail: '' };
    }
  }
  return {
    dir: '',
    detail:
      'the iii-directory plugin is not built (run `npm run build:directory-plugin`); agent profiles and skills are not available over MCP',
  };
}
