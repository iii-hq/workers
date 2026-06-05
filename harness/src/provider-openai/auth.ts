/**
 * Resolve the OpenAI credential + runtime settings from the harness provider
 * registry (`harness::provider::resolve`) and build a ChatCompletionsConfig.
 */

import type { ISdk } from '../runtime/iii.js';
import { clampOutputTokens, getCatalogModel } from '../runtime/output-tokens.js';
import { resolveProvider } from '../runtime/provider-resolve.js';
import type { WorkerConfig } from './config.js';
import { type ChatCompletionsConfig, configFromCredential } from './types.js';

export const PROVIDER_ID = 'openai';

export async function buildConfig(
  iii: ISdk,
  worker: WorkerConfig,
  model: string,
): Promise<ChatCompletionsConfig> {
  const resolved = await resolveProvider(iii, PROVIDER_ID);
  if (!resolved.credential) {
    throw new Error(
      'harness::provider::resolve returned no credential for provider `openai` ' +
        '(set an api key in the harness configuration or OPENAI_API_KEY)',
    );
  }
  const apiUrl = resolved.api_url ?? worker.default_api_url;
  const catalog = await getCatalogModel(iii, PROVIDER_ID, model);
  const maxTokens = clampOutputTokens({
    modelMaxOutput: catalog?.max_output_tokens,
    userOverride: resolved.max_tokens,
    workerDefault: worker.default_max_tokens,
  });
  const cfg = configFromCredential(apiUrl, PROVIDER_ID, model, resolved.credential, maxTokens);
  return catalog ? { ...cfg, catalog } : cfg;
}
