import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import type { OverridesStore } from '../store.js';
import type { ProviderOverrides } from '../types.js';

const MAX_TOKENS_LIMIT = 1_048_576;

function validateUrl(raw: unknown): string {
  if (typeof raw !== 'string' || raw.length === 0) {
    throw new Error('default_api_url must be a non-empty string');
  }
  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    throw new Error(`default_api_url is not a valid URL: ${raw}`);
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`default_api_url must use http or https scheme, got: ${parsed.protocol}`);
  }
  // Parity with normalizeChatCompletionsUrl: reject empty-hostname URLs like
  // `http:///path`. Without this, set() accepts URLs that the normaliser
  // later rejects, producing inconsistent failure modes.
  if (parsed.hostname.length === 0) {
    throw new Error('default_api_url must include a hostname');
  }
  return raw;
}

function validateMaxTokens(raw: unknown): number {
  if (typeof raw !== 'number' || !Number.isInteger(raw)) {
    throw new Error('default_max_tokens must be an integer');
  }
  if (raw <= 0) {
    throw new Error('default_max_tokens must be a positive integer');
  }
  if (raw > MAX_TOKENS_LIMIT) {
    throw new Error(`default_max_tokens must be <= ${MAX_TOKENS_LIMIT}`);
  }
  return raw;
}

export function register(iii: ISdk, store: OverridesStore): void {
  iii.registerFunction(
    'provider_config::set',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');

      const hasUrl = obj.default_api_url !== undefined;
      const hasMaxTokens = obj.default_max_tokens !== undefined;
      if (!hasUrl && !hasMaxTokens) {
        throw new Error('at least one of default_api_url or default_max_tokens must be provided');
      }

      const overrides: ProviderOverrides = {};
      if (hasUrl) {
        overrides.default_api_url = validateUrl(obj.default_api_url);
      }
      if (hasMaxTokens) {
        overrides.default_max_tokens = validateMaxTokens(obj.default_max_tokens);
      }

      await store.set(provider, overrides);
      return { ok: true };
    },
    { description: 'Set runtime overrides for a provider.' },
  );
}
