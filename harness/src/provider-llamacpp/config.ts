import { logger } from '../runtime/otel.js';
import { getNumber, getSection, getString } from '../runtime/config.js';

export type WorkerConfig = {
  default_max_tokens: number;
  default_api_url: string;
};

// llama-server's default port is 8080. Anyone running multiple inference
// servers will likely override this — see config.yaml / LLAMACPP_BASE_URL.
export const DEFAULT_API_URL = 'http://localhost:8080/v1/chat/completions';

/**
 * Validate a candidate URL: parses cleanly AND uses an http/https
 * scheme. Returns the parsed URL if valid, null otherwise.
 *
 * Same security posture as provider-lmstudio: a malformed or
 * attacker-controlled `LLAMACPP_BASE_URL` (stale EnvironmentFile, typo
 * with extra leading scheme) is rejected so we never concatenate an
 * unvalidated string into the request target.
 */
function validatedUrl(raw: string): URL | null {
  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    return null;
  }
  return parsed;
}

function isLoopbackHost(host: string): boolean {
  const h = host.toLowerCase();
  if (h === 'localhost' || h.endsWith('.localhost')) return true;
  if (h === '127.0.0.1' || h === '::1' || h === '[::1]') return true;
  if (/^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(h)) return true;
  return false;
}

/**
 * Resolve the llama-server base URL with this precedence:
 *   1. `LLAMACPP_BASE_URL` env var (per-machine override — wins over yaml)
 *   2. `provider_llamacpp.default_api_url` in config.yaml
 *   3. The localhost DEFAULT_API_URL constant above
 *
 * The env var accepts either a base origin (`http://host:port`) or a
 * full URL (`http://host:port/v1/chat/completions`). When only a base
 * is given, `/v1/chat/completions` is appended automatically.
 *
 * Both the env value and the yaml value are validated as http(s) URLs;
 * a malformed value falls back to the next tier rather than being
 * concatenated raw. A warning is logged when the resolved host is not
 * loopback so operators see they're shipping their bearer to a remote.
 */
function resolveApiUrl(yamlValue: string): string {
  const envRaw = (process.env.LLAMACPP_BASE_URL ?? '').trim();
  let candidate: string | null = null;
  if (envRaw.length > 0) {
    const trimmedEnv = envRaw.endsWith('/') ? envRaw.slice(0, -1) : envRaw;
    const withPath = trimmedEnv.includes('/chat/completions')
      ? trimmedEnv
      : `${trimmedEnv}/v1/chat/completions`;
    if (validatedUrl(withPath)) {
      candidate = withPath;
    } else {
      logger.warn(
        'llamacpp.config: LLAMACPP_BASE_URL is not a valid http(s) URL — ignoring',
        { code: 'llamacpp_base_url_invalid' },
      );
    }
  }
  if (candidate === null) candidate = yamlValue;
  const parsed = validatedUrl(candidate);
  if (!parsed) {
    logger.warn(
      'llamacpp.config: resolved API URL is not a valid http(s) URL; falling back to default',
      { code: 'llamacpp_api_url_invalid' },
    );
    return DEFAULT_API_URL;
  }
  if (!isLoopbackHost(parsed.hostname)) {
    logger.warn(
      'llamacpp.config: API URL is non-loopback — bearer (if configured) will be sent to a remote host',
      {
        code: 'llamacpp_api_url_remote',
        origin: `${parsed.protocol}//${parsed.host}`,
      },
    );
  }
  return candidate;
}

export function loadWorkerConfig(cfg: Record<string, unknown>): WorkerConfig {
  const section = getSection(cfg, 'provider_llamacpp');
  const yamlUrl = getString(section, 'default_api_url', DEFAULT_API_URL);
  return {
    default_max_tokens: getNumber(section, 'default_max_tokens', 8192),
    default_api_url: resolveApiUrl(yamlUrl),
  };
}
