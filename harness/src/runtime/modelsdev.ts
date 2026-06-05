/**
 * models.dev catalog client.
 *
 * Fetches https://models.dev/api.json once per process (1h TTL, in-flight
 * dedup) and exposes per-provider/model limits + capability flags used to
 * enrich discovery. Best-effort by contract: any failure yields an empty
 * index so discovery proceeds with its existing per-provider defaults.
 */

import { logger } from './otel.js';

export type ModelsDevLimit = {
  context?: number;
  input?: number;
  output?: number;
};

export type ModelsDevModel = {
  id: string;
  limit?: ModelsDevLimit;
  reasoning?: boolean;
  tool_call?: boolean;
};

/** providerKey -> (modelId -> ModelsDevModel) */
export type ModelsDevIndex = Map<string, Map<string, ModelsDevModel>>;

export const MODELSDEV_URL = 'https://models.dev/api.json';
const FETCH_TIMEOUT_MS = 8_000;
const CACHE_TTL_MS = 60 * 60 * 1000;
// Failed fetches cache an empty index, but only briefly — a transient blip
// at startup shouldn't strip catalog limits for a full hour.
const FAILURE_TTL_MS = 5 * 60 * 1000;
// Sanity bounds for externally-sourced token limits. models.dev is a
// third-party feed; an absurd value (e.g. output: 1) would silently break
// every request to that model, so out-of-range values are treated as absent.
const LIMIT_MIN = 256;
const LIMIT_MAX = 50_000_000;

/** Harness provider id -> models.dev provider key. Local providers have no entry. */
export const PROVIDER_KEY_MAP: Record<string, string> = {
  anthropic: 'anthropic',
  openai: 'openai',
  kimi: 'moonshotai',
};

type CacheEntry = { index: ModelsDevIndex; fetchedAt: number };

let cache: CacheEntry | null = null;
let inflight: Promise<ModelsDevIndex> | null = null;

/** Test hook: clear the module cache. */
export function _resetModelsDevForTests(): void {
  cache = null;
  inflight = null;
}

function emptyIndex(): ModelsDevIndex {
  return new Map();
}

function num(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) && v >= LIMIT_MIN && v <= LIMIT_MAX
    ? v
    : undefined;
}

function parseIndex(json: unknown): ModelsDevIndex {
  const index = emptyIndex();
  if (typeof json !== 'object' || json === null) return index;
  for (const [providerKey, provider] of Object.entries(json as Record<string, unknown>)) {
    if (typeof provider !== 'object' || provider === null) continue;
    const models = (provider as Record<string, unknown>).models;
    if (typeof models !== 'object' || models === null) continue;
    const byId = new Map<string, ModelsDevModel>();
    for (const [modelId, raw] of Object.entries(models as Record<string, unknown>)) {
      if (typeof raw !== 'object' || raw === null) continue;
      const m = raw as Record<string, unknown>;
      const limit =
        typeof m.limit === 'object' && m.limit !== null
          ? (m.limit as Record<string, unknown>)
          : undefined;
      byId.set(modelId, {
        id: modelId,
        limit: limit
          ? { context: num(limit.context), input: num(limit.input), output: num(limit.output) }
          : undefined,
        reasoning: typeof m.reasoning === 'boolean' ? m.reasoning : undefined,
        tool_call: typeof m.tool_call === 'boolean' ? m.tool_call : undefined,
      });
    }
    if (byId.size > 0) index.set(providerKey, byId);
  }
  return index;
}

async function fetchIndex(): Promise<ModelsDevIndex> {
  const ac = new AbortController();
  const timer = setTimeout(() => ac.abort(), FETCH_TIMEOUT_MS);
  try {
    const resp = await fetch(MODELSDEV_URL, { signal: ac.signal });
    if (!resp.ok) {
      logger.warn('modelsdev: non-200 response', { status: resp.status });
      return emptyIndex();
    }
    const json: unknown = await resp.json();
    const index = parseIndex(json);
    logger.info('modelsdev: catalog fetched', { providers: index.size });
    return index;
  } catch (err) {
    logger.warn('modelsdev: fetch failed', { err: String(err) });
    return emptyIndex();
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Cached models.dev index. Never throws — failures cache an empty index
 * (with a shorter TTL) so a flaky network can't hammer models.dev or block
 * discovery, while a transient blip recovers within minutes.
 */
export async function getModelsDevIndex(): Promise<ModelsDevIndex> {
  if (cache) {
    const ttl = cache.index.size > 0 ? CACHE_TTL_MS : FAILURE_TTL_MS;
    if (Date.now() - cache.fetchedAt < ttl) return cache.index;
  }
  if (inflight) return inflight;
  inflight = fetchIndex().then((index) => {
    cache = { index, fetchedAt: Date.now() };
    inflight = null;
    return index;
  });
  return inflight;
}

/** Strip a trailing `-YYYYMMDD` date suffix (`claude-sonnet-4-20250514` -> `claude-sonnet-4`). */
function stripDateSuffix(id: string): string {
  return id.replace(/-\d{8}$/, '');
}

/**
 * Look up a model's models.dev metadata for a harness provider id.
 * Conservative matching: exact id, then date-suffix-normalized on both sides.
 * No match -> undefined (caller keeps its defaults).
 */
export function lookupModelsDev(
  index: ModelsDevIndex,
  providerId: string,
  modelId: string,
): ModelsDevModel | undefined {
  const key = PROVIDER_KEY_MAP[providerId];
  if (!key) return undefined;
  const models = index.get(key);
  if (!models) return undefined;

  const exact = models.get(modelId);
  if (exact) return exact;

  const normalized = stripDateSuffix(modelId);
  if (normalized !== modelId) {
    const byNormalized = models.get(normalized);
    if (byNormalized) return byNormalized;
  }
  for (const [id, model] of models) {
    if (stripDateSuffix(id) === normalized) return model;
  }
  return undefined;
}
