/**
 * Kimi (Moonshot) model discovery — hits the OpenAI-compatible
 * `GET /v1/models` and registers each returned model into the iii models
 * catalog (cached for the picker) with a default context window.
 *
 * Best-effort: a missing credential or any upstream error yields `[]`.
 */

import type { ISdk } from '../runtime/iii.js';
import {
  deriveModelsUrl,
  enrichModel,
  fetchModelsJson,
  type ModelStub,
  registerModels,
} from '../runtime/models-discovery.js';
import { logger } from '../runtime/otel.js';
import { resolveProvider } from '../runtime/provider-resolve.js';
import { PROVIDER_ID } from './auth.js';
import type { WorkerConfig } from './config.js';

const DEFAULT_CONTEXT_WINDOW = 256_000;

function parseStubs(json: unknown): ModelStub[] {
  const data = (json as { data?: unknown })?.data;
  if (!Array.isArray(data)) return [];
  const out: ModelStub[] = [];
  for (const raw of data as Array<{ id?: unknown }>) {
    const id = typeof raw.id === 'string' && raw.id.length > 0 ? raw.id : null;
    if (!id) continue;
    out.push({ id });
  }
  return out;
}

export async function discoverAndRegister(iii: ISdk, worker: WorkerConfig): Promise<string[]> {
  const resolved = await resolveProvider(iii, PROVIDER_ID).catch(() => null);
  const cred = resolved?.credential ?? null;
  if (!cred) {
    logger.info('kimi discovery: no credential; skipping', {});
    return [];
  }
  const key = cred.type === 'api_key' ? cred.key : cred.access_token;
  const url = deriveModelsUrl(resolved?.api_url ?? worker.default_api_url);
  const json = await fetchModelsJson(url, { Authorization: `Bearer ${key}` });
  if (!json) return [];

  const models = parseStubs(json).map((stub) =>
    enrichModel({
      provider: PROVIDER_ID,
      api: 'openai-completions',
      stub,
      defaultContextWindow: DEFAULT_CONTEXT_WINDOW,
    }),
  );
  if (models.length === 0) return [];
  const registered = await registerModels(iii, models);
  logger.info('kimi discovery: registered models', { count: registered.length });
  return registered;
}
