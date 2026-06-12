/**
 * Resolve the Kimi (Moonshot) credential + runtime settings from the llm-router
 * registry (token-gated `router::provider::resolve`) and build a
 * ChatCompletionsConfig. The provider id is `kimi`; the registry's env-var
 * fallback maps it to MOONSHOT_API_KEY.
 */

import type { ISdk } from '../runtime/iii.js';
import { clampOutputTokens, getCatalogModel } from '../runtime/output-tokens.js';
import { resolveProviderViaRouter } from '../runtime/provider-resolve.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

export const PROVIDER_ID = 'kimi';

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
  maxOutputOverride?: number,
): Promise<ChatCompletionsConfig> {
  const resolved = await resolveProviderViaRouter(iii, PROVIDER_ID);
  if (!resolved.credential) {
    throw new Error(
      'router::provider::resolve returned no credential for provider `kimi` ' +
        '(set an api key in the llm-router configuration or MOONSHOT_API_KEY)',
    );
  }
  const apiUrl = resolved.api_url ?? worker.default_api_url;
  const catalog = await getCatalogModel(iii, PROVIDER_ID, model);
  const maxTokens = clampOutputTokens({
    modelMaxOutput: catalog?.max_output_tokens,
    // The router's resolved budget (when threaded) is authoritative; it still
    // runs through the model-ceiling clamp as the override, never raised.
    userOverride: maxOutputOverride ?? resolved.max_tokens,
    workerDefault: worker.default_max_tokens,
  });
  return configFromCredential(apiUrl, PROVIDER_ID, model, resolved.credential, maxTokens);
}
