/**
 * Resolve the OpenAI credential + runtime settings from the llm-router
 * registry (token-gated `router::provider::resolve`) and build a ChatCompletionsConfig.
 */

import type { ISdk } from '../runtime/iii.js';
import { clampOutputTokens, getCatalogModel } from '../runtime/output-tokens.js';
import { resolveProviderViaRouter } from '../runtime/provider-resolve.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

export const PROVIDER_ID = 'openai';

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
  maxOutputOverride?: number,
): Promise<ChatCompletionsConfig> {
  const resolved = await resolveProviderViaRouter(iii, PROVIDER_ID);
  if (!resolved.credential) {
    throw new Error(
      'router::provider::resolve returned no credential for provider `openai` ' +
        '(set an api key in the llm-router configuration or OPENAI_API_KEY)',
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
  const cfg = configFromCredential(apiUrl, PROVIDER_ID, model, resolved.credential, maxTokens);
  return catalog ? { ...cfg, catalog } : cfg;
}
