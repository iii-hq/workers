/**
 * OpenAI model discovery — hits `GET /v1/models` and registers the
 * chat-capable subset into the iii models catalog (cached for the picker).
 * The OpenAI list includes embeddings/audio/image models; we filter to
 * chat/reasoning families and register them with a default context window.
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

const DEFAULT_CONTEXT_WINDOW = 128_000;

// Chat/reasoning families we route to. Excludes embeddings, audio, image,
// moderation, realtime, and legacy completion-only model ids.
const CHAT_ID = /^(gpt-|o\d|chatgpt)/i;
const NON_CHAT =
  /(embedding|whisper|tts|audio|dall-e|image|moderation|realtime|transcribe|search|babbage|davinci|ada|curie)/i;

function parseStubs(json: unknown): ModelStub[] {
  const data = (json as { data?: unknown })?.data;
  if (!Array.isArray(data)) return [];
  const out: ModelStub[] = [];
  for (const raw of data as Array<{ id?: unknown }>) {
    const id = typeof raw.id === 'string' && raw.id.length > 0 ? raw.id : null;
    if (!id) continue;
    if (!CHAT_ID.test(id) || NON_CHAT.test(id)) continue;
    out.push({ id });
  }
  return out;
}

export async function discoverAndRegister(iii: ISdk, worker: WorkerConfig): Promise<string[]> {
  const resolved = await resolveProvider(iii, PROVIDER_ID).catch(() => null);
  const cred = resolved?.credential ?? null;
  if (!cred) {
    logger.info('openai discovery: no credential; skipping', {});
    return [];
  }
  const key = cred.type === 'api_key' ? cred.key : cred.access_token;
  const url = deriveModelsUrl(resolved?.api_url ?? worker.default_api_url);
  const json = await fetchModelsJson(url, { Authorization: `Bearer ${key}` });
  if (!json) return [];

  const models = parseStubs(json).map((stub) =>
    enrichModel({
      provider: PROVIDER_ID,
      api: 'openai-responses',
      stub,
      defaultContextWindow: DEFAULT_CONTEXT_WINDOW,
    }),
  );
  if (models.length === 0) return [];
  const registered = await registerModels(iii, models);
  logger.info('openai discovery: registered models', { count: registered.length });
  return registered;
}
