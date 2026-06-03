/**
 * State-backed reads for the models catalog. Each provider owns one key in
 * scope `models` whose value is a `Model[]`. Discovery writes via
 * `models::reconcile` (single state set per provider).
 */

import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateListValues } from '../runtime/state.js';
import { type ListFilter, MODELS_SCOPE, type Model, supportsModel } from './types.js';

/** State key for a provider's catalog array (scope `models`, key = provider id). */
export function providerStateKey(provider: string): string {
  return provider;
}

export function isModel(v: unknown): v is Model {
  return Boolean(v && typeof v === 'object' && typeof (v as Model).id === 'string');
}

/** Parse a stored catalog value; only `Model[]` is valid. */
export function parseModelArray(v: unknown): Model[] {
  if (!Array.isArray(v)) return [];
  return v.filter(isModel);
}

export async function getProviderModels(iii: ISdk, provider: string): Promise<Model[]> {
  const v = await stateGet(iii, MODELS_SCOPE, providerStateKey(provider));
  return parseModelArray(v);
}

export async function listFromState(iii: ISdk, filter: ListFilter): Promise<Model[]> {
  if (filter.provider !== undefined) {
    const models = await getProviderModels(iii, filter.provider);
    return filter.capability === undefined
      ? models
      : models.filter((m) => supportsModel(m, filter.capability!));
  }

  const entries = await stateListValues<unknown>(iii, { scope: MODELS_SCOPE });
  const out: Model[] = [];
  for (const entry of entries) {
    out.push(...parseModelArray(entry));
  }
  return filter.capability === undefined
    ? out
    : out.filter((m) => supportsModel(m, filter.capability!));
}

export async function getFromState(iii: ISdk, provider: string, id: string): Promise<Model | null> {
  const models = await getProviderModels(iii, provider);
  return models.find((m) => m.id === id) ?? null;
}
