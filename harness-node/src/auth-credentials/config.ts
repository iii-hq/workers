import { getSection, getString } from '../runtime/config.js';

export type AuthCredentialsConfig = {
  store_path: string;
};

export const DEFAULT_STORE_PATH = '~/.iii/auth-credentials.json';

export function loadAuthCredentialsConfig(cfg: Record<string, unknown>): AuthCredentialsConfig {
  const section = getSection(cfg, 'auth_credentials');
  return {
    store_path: getString(section, 'store_path', DEFAULT_STORE_PATH),
  };
}
