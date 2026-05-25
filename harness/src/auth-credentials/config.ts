import { getSection } from '../runtime/config.js';
import { loadStorageConfig, resolveDatabaseName } from '../runtime/storage-config.js';

export type AuthCredentialsConfig = {
  /** Logical database pool name to use (must match a key in iii-database's `databases:` map). */
  database_name: string;
};

export function loadAuthCredentialsConfig(cfg: Record<string, unknown>): AuthCredentialsConfig {
  const section = getSection(cfg, 'auth_credentials');
  const storage = loadStorageConfig(cfg);
  return {
    database_name: resolveDatabaseName(section, storage),
  };
}
