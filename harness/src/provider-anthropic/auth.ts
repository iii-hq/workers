/**
 * Resolve the Anthropic credential + runtime settings from the harness
 * provider registry (`harness::provider::resolve`), then turn them into an
 * AnthropicConfig. Replaces the old `auth::get_token` + `provider_config::get`
 * pair with a single call.
 */

import type { ISdk } from '../runtime/iii.js';
import { resolveProvider } from '../runtime/provider-resolve.js';
import type { WorkerConfig } from './config.js';
import { type AnthropicConfig, configWithCredential } from './types.js';

export const PROVIDER_ID = 'anthropic';

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<AnthropicConfig> {
  const resolved = await resolveProvider(iii, PROVIDER_ID);
  if (!resolved.credential) {
    throw new Error(
      'harness::provider::resolve returned no credential for provider `anthropic` ' +
        '(set an api key in the harness configuration or ANTHROPIC_API_KEY)',
    );
  }
  const apiUrl = resolved.api_url ?? worker.default_api_url;
  const maxTokens = resolved.max_tokens ?? worker.default_max_tokens;
  return configWithCredential(model, resolved.credential, maxTokens, apiUrl);
}
