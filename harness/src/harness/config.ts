import { getString } from '../runtime/config.js';

export type HarnessConfig = {
  engine_url: string;
  permissions_path: string;
};

export function loadHarnessConfig(cfg: Record<string, unknown>): HarnessConfig {
  return {
    engine_url: getString(cfg, 'engine_url', 'ws://127.0.0.1:49134'),
    permissions_path: getString(cfg, 'permissions_path', './iii-permissions.yaml'),
  };
}
