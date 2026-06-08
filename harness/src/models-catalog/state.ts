/**
 * State-backed reads for the models catalog: scope `models`, one Model[] per
 * provider key. `models::reconcile` validates entries via isModel before writing
 * and state is cleared on deploy, so reads trust the stored shape (no re-parsing).
 */

import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateListValues } from '../runtime/state.js';
import { type ListFilter, MODELS_SCOPE, type Model, supportsModel } from './types.js';

export function providerStateKey(provider: string): string {
  return provider;
}

/** Write-side boundary guard for provider-discovery output (used by models::reconcile). */
export function isModel(v: unknown): v is Model {
  return Boolean(v && typeof v === 'object' && typeof (v as Model).id === 'string');
}

export async function getProviderModels(iii: ISdk, provider: string): Promise<Model[]> {
  return (await stateGet<Model[]>(iii, MODELS_SCOPE, providerStateKey(provider))) ?? [];
}

export async function listFromState(iii: ISdk, filter: ListFilter): Promise<Model[]> {
  if (filter.provider !== undefined) {
    const models = await getProviderModels(iii, filter.provider);
    return filter.capability === undefined
      ? models
      : models.filter((m) => supportsModel(m, filter.capability!));
  }

  // Each provider key stores one Model[]; flatten across providers.
  const out = (await stateListValues<Model[]>(iii, { scope: MODELS_SCOPE })).flat();
  return filter.capability === undefined
    ? out
    : out.filter((m) => supportsModel(m, filter.capability!));
}

export async function getFromState(iii: ISdk, provider: string, id: string): Promise<Model | null> {
  const models = await getProviderModels(iii, provider);
  return models.find((m) => m.id === id) ?? null;
}
