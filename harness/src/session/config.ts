import { getSection, getString } from '../runtime/config.js';

export type SessionConfig = {
  store_backend: 'memory' | 'iii_state';
  state_scope: string;
};

export function loadSessionConfig(cfg: Record<string, unknown>): SessionConfig {
  const section = getSection(cfg, 'session');
  const backend = getString(section, 'store_backend', 'iii_state');
  return {
    store_backend: backend === 'memory' ? 'memory' : 'iii_state',
    state_scope: getString(section, 'state_scope', 'agent'),
  };
}
