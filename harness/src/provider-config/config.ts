import { getSection } from '../runtime/config.js';
import { loadStorageConfig, resolveDatabaseName } from '../runtime/storage-config.js';

export type ProviderConfigConfig = {
  /** Logical database pool name to use (must match a key in iii-database's `databases:` map). */
  database_name: string;
};

export function loadProviderConfigConfig(cfg: Record<string, unknown>): ProviderConfigConfig {
  const section = getSection(cfg, 'provider_config');
  const storage = loadStorageConfig(cfg);
  return {
    database_name: resolveDatabaseName(section, storage),
  };
}
