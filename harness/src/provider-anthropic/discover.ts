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
  fetchModelsForDiscovery,
  type ModelStub,
  reconcileModels,
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
    // No credential: drop any models a previous run registered so the picker
    // reflects the removal instead of showing stale, unusable rows.
    logger.info('anthropic discovery: no credential; pruning catalog', {});
    await reconcileModels(iii, PROVIDER_ID, []);
    return [];
  }
  const key = cred.type === 'api_key' ? cred.key : cred.access_token;
  const url = deriveModelsUrl(resolved?.api_url ?? worker.default_api_url);
  const fetchResult = await fetchModelsForDiscovery(url, {
    'x-api-key': key,
    'anthropic-version': ANTHROPIC_VERSION,
  });
  if (fetchResult.kind === 'auth_error') {
    logger.info('anthropic discovery: invalid credential; pruning catalog', {
      status: fetchResult.status,
    });
    await reconcileModels(iii, PROVIDER_ID, []);
    return [];
  }
  if (fetchResult.kind !== 'ok') return [];

  const models = parseStubs(fetchResult.json).map((stub) =>
    enrichModel({
      provider: PROVIDER_ID,
      api: 'anthropic-messages',
      stub,
      defaultContextWindow: DEFAULT_CONTEXT_WINDOW,
    }),
  );
  const registered = await reconcileModels(iii, PROVIDER_ID, models);
  logger.info('anthropic discovery: reconciled models', {
    count: registered.length,
    discovered: models.length,
  });
  return registered;
}
