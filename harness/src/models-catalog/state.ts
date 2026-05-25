/**
 * State-backed reads + seed. Mirrors
 * `models-catalog/src/lib.rs::{seed_state_if_empty, list_from_state_or_seed,
 *  get_from_state_or_seed}`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { stateGet, stateListValues, stateSet } from '../runtime/state.js';
import { type ListFilter, loadEmbeddedCatalog } from './catalog.js';
import { MODELS_KEY_PREFIX, MODELS_SCOPE, type Model, supportsModel } from './types.js';

export type StateConfig = {
  state_request_timeout_ms: number;
};

export const DEFAULT_STATE_CONFIG: StateConfig = { state_request_timeout_ms: 5_000 };

export function modelKey(provider: string, id: string): string {
  return `${MODELS_KEY_PREFIX}${provider}:${id}`;
}

export async function seedStateIfEmpty(iii: ISdk, _cfg: StateConfig): Promise<void> {
  try {
    const items = await stateListValues<Model>(iii, { scope: MODELS_SCOPE });
    if (items.length > 0) return;
    const catalog = await loadEmbeddedCatalog();
    for (const m of catalog) {
      await stateSet(iii, MODELS_SCOPE, modelKey(m.provider, m.id), m);
    }
    logger.info('models-catalog: seeded embedded baseline into state', {
      count: catalog.length,
    });
  } catch (err) {
    logger.warn('models-catalog: seed failed', { err: String(err) });
  }
}

export async function listFromStateOrSeed(iii: ISdk, filter: ListFilter): Promise<Model[]> {
  const fromState = (await stateListValues<Model>(iii, { scope: MODELS_SCOPE })).filter(
    (m): m is Model => Boolean(m && typeof m === 'object' && m.id),
  );
  const source = fromState.length > 0 ? fromState : await loadEmbeddedCatalog();
  return source
    .filter((m) => filter.provider === undefined || m.provider === filter.provider)
    .filter((m) => filter.capability === undefined || supportsModel(m, filter.capability));
}

export async function getFromStateOrSeed(
  iii: ISdk,
  provider: string,
  id: string,
): Promise<Model | null> {
  const v = await stateGet(iii, MODELS_SCOPE, modelKey(provider, id));
  if (v && typeof v === 'object') return v as Model;
  const catalog = await loadEmbeddedCatalog();
  return catalog.find((m) => m.provider === provider && m.id === id) ?? null;
}
