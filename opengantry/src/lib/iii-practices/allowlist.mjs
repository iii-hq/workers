/**
 * Planner-controlled HTTP connector allowlist for iii-architecture pragma ratchet.
 * Path: <repo-root>/.gitagent/planner/iii-architecture.allowlist.json
 * Override for tests: GANTRY_III_ARCH_ALLOWLIST=/path/to/allowlist.json
 */
import fs from 'node:fs';
import path from 'node:path';

export const ALLOWLIST_REL = '.gitagent/planner/iii-architecture.allowlist.json';
export const HTTP_PRAGMA = 'gantry-allow-external-http';

/**
 * Resolve OpenGantry repo root (hosts .gitagent/planner allowlist).
 * --repo-root wins; else walk up from cwd for ALLOWLIST_REL.
 */
export function resolveRepoRoot(argv = process.argv) {
  const idx = argv.indexOf('--repo-root');
  if (idx >= 0 && argv[idx + 1]) {
    return path.resolve(argv[idx + 1]);
  }
  let dir = process.cwd();
  for (;;) {
    if (fs.existsSync(path.join(dir, ALLOWLIST_REL))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return process.cwd();
}

/**
 * @returns {{ workers: Set<string>, path: string }}
 */
export function loadHttpConnectorAllowlist(repoRoot) {
  const envPath = process.env.GANTRY_III_ARCH_ALLOWLIST;
  const allowlistPath = envPath ? path.resolve(envPath) : path.join(repoRoot, ALLOWLIST_REL);
  if (!fs.existsSync(allowlistPath)) {
    return { workers: new Set(), path: allowlistPath };
  }
  let raw;
  try {
    raw = JSON.parse(fs.readFileSync(allowlistPath, 'utf8'));
  } catch (e) {
    const err = new Error(`invalid allowlist JSON (${allowlistPath}): ${e.message}`);
    err.code = 'INVALID_ALLOWLIST';
    throw err;
  }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    const err = new Error(`invalid allowlist: root must be a JSON object (${allowlistPath})`);
    err.code = 'INVALID_ALLOWLIST';
    throw err;
  }
  const list = raw.http_connector_workers;
  if (!Array.isArray(list)) {
    const err = new Error(
      `invalid allowlist: http_connector_workers must be an array (${allowlistPath})`,
    );
    err.code = 'INVALID_ALLOWLIST';
    throw err;
  }
  return { workers: new Set(list.filter((w) => typeof w === 'string')), path: allowlistPath };
}
