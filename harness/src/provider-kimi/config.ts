import { getNumber, getSection, getString } from '../runtime/config.js';

export type WorkerConfig = {
  default_max_tokens: number;
  default_api_url: string;
};

export const DEFAULT_API_URL = 'https://api.moonshot.ai/v1/chat/completions';

export function loadWorkerConfig(cfg: Record<string, unknown>): WorkerConfig {
  const section = getSection(cfg, 'provider_kimi');
  return {
    default_max_tokens: getNumber(section, 'default_max_tokens', 8192),
    default_api_url: getString(section, 'default_api_url', DEFAULT_API_URL),
  };
}
