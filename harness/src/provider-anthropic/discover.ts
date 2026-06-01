/**
 * Anthropic model discovery — hits `GET /v1/models` and registers each
 * returned model into the iii models catalog so the picker shows the live
 * list (cached), with a default context window + conservative capability
 * flags (upstream `/v1/models` exposes little metadata).
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

const ANTHROPIC_VERSION = '2023-06-01';
const DEFAULT_CONTEXT_WINDOW = 200_000;

type AnthropicModel = { id?: unknown; display_name?: unknown };

function parseStubs(json: unknown): ModelStub[] {
  const data = (json as { data?: unknown })?.data;
  if (!Array.isArray(data)) return [];
  const out: ModelStub[] = [];
  for (const raw of data as AnthropicModel[]) {
    const id = typeof raw.id === 'string' && raw.id.length > 0 ? raw.id : null;
    if (!id) continue;
    out.push({
      id,
      display_name: typeof raw.display_name === 'string' ? raw.display_name : undefined,
    });
  }
  return out;
}

export async function discoverAndRegister(iii: ISdk, worker: WorkerConfig): Promise<string[]> {
  const resolved = await resolveProvider(iii, PROVIDER_ID).catch(() => null);
  const cred = resolved?.credential ?? null;
  if (!cred) {
    logger.info('anthropic discovery: no credential; skipping', {});
    return [];
  }
  const key = cred.type === 'api_key' ? cred.key : cred.access_token;
  const url = deriveModelsUrl(resolved?.api_url ?? worker.default_api_url);
  const json = await fetchModelsJson(url, {
    'x-api-key': key,
    'anthropic-version': ANTHROPIC_VERSION,
  });
  if (!json) return [];

  const models = parseStubs(json).map((stub) =>
    enrichModel({
      provider: PROVIDER_ID,
      api: 'anthropic-messages',
      stub,
      defaultContextWindow: DEFAULT_CONTEXT_WINDOW,
    }),
  );
  if (models.length === 0) return [];
  const registered = await registerModels(iii, models);
  logger.info('anthropic discovery: registered models', { count: registered.length });
  return registered;
}
