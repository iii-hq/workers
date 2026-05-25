/**
 * Shared harness storage configuration. Every worker that talks to
 * `database::*` resolves its logical pool name through this helper so
 * operators can set `storage.database_name` once in config.yaml.
 */

import { getSection, getString } from './config.js';

export const DEFAULT_DATABASE_NAME = 'harness';

export type StorageConfig = {
  /** Logical database pool name (must match a key in the engine `database` worker's `databases:` map). */
  database_name: string;
};

/** Read the top-level `storage:` section from an already-loaded config object. */
export function loadStorageConfig(cfg: Record<string, unknown>): StorageConfig {
  const section = getSection(cfg, 'storage');
  return {
    database_name: getString(section, 'database_name', DEFAULT_DATABASE_NAME),
  };
}

/**
 * Resolve the effective database pool name for a worker.
 *
 * Precedence: worker section `database_name` > `storage.database_name` > `"harness"`.
 */
export function resolveDatabaseName(
  workerSection: Record<string, unknown>,
  storage: StorageConfig,
): string {
  const workerOverride = optionalString(workerSection, 'database_name');
  if (workerOverride) return workerOverride;
  return storage.database_name;
}

function optionalString(cfg: Record<string, unknown>, key: string): string | undefined {
  const v = cfg[key];
  return typeof v === 'string' && v.length > 0 ? v : undefined;
}
