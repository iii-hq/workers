import { getNumber, getSection, getString } from '../runtime/config.js';

export type WorkerConfig = {
  default_max_tokens: number;
  default_api_url: string;
};

export const DEFAULT_API_URL = 'http://localhost:1234/v1/chat/completions';

/**
 * Resolve the LM Studio base URL with this precedence:
 *   1. `LMSTUDIO_BASE_URL` env var (per-machine override — wins over yaml)
 *   2. `provider_lmstudio.default_api_url` in config.yaml
 *   3. The localhost DEFAULT_API_URL constant above
 *
 * The env var accepts either a base origin (`http://host:port`) or a full
 * URL (`http://host:port/v1/chat/completions`). When only a base is given,
 * `/v1/chat/completions` is appended automatically.
 */
function resolveApiUrl(yamlValue: string): string {
  const env = (process.env.LMSTUDIO_BASE_URL ?? '').trim();
  if (env.length === 0) return yamlValue;
  if (env.includes('/chat/completions')) return env;
  const trimmed = env.endsWith('/') ? env.slice(0, -1) : env;
  return `${trimmed}/v1/chat/completions`;
}

export function loadWorkerConfig(cfg: Record<string, unknown>): WorkerConfig {
  const section = getSection(cfg, 'provider_lmstudio');
  const yamlUrl = getString(section, 'default_api_url', DEFAULT_API_URL);
  return {
    default_max_tokens: getNumber(section, 'default_max_tokens', 8192),
    default_api_url: resolveApiUrl(yamlUrl),
  };
}
