/**
 * State-backed reads for the models catalog. The catalog is populated
 * exclusively by provider discovery (`models::register`) — there is no
 * embedded seed or fallback. `models::list` / `models::get` return only what
 * providers have registered into iii state.
 */

import type { ISdk } from '../runtime/iii.js';
import { stateGet, stateListValues } from '../runtime/state.js';
import {
  type ListFilter,
  MODELS_KEY_PREFIX,
  MODELS_SCOPE,
  type Model,
  supportsModel,
} from './types.js';

export function modelKey(provider: string, id: string): string {
  return `${MODELS_KEY_PREFIX}${provider}:${id}`;
}

export async function listFromState(iii: ISdk, filter: ListFilter): Promise<Model[]> {
  const models = (await stateListValues<Model>(iii, { scope: MODELS_SCOPE })).filter(
    (m): m is Model => Boolean(m && typeof m === 'object' && m.id),
  );
  return models
    .filter((m) => filter.provider === undefined || m.provider === filter.provider)
    .filter((m) => filter.capability === undefined || supportsModel(m, filter.capability));
}

export async function getFromState(iii: ISdk, provider: string, id: string): Promise<Model | null> {
  const v = await stateGet(iii, MODELS_SCOPE, modelKey(provider, id));
  return v && typeof v === 'object' ? (v as Model) : null;
}
