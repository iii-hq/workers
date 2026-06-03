import type { ISdk } from '../../runtime/iii.js';
import { stateSet } from '../../runtime/state.js';
import { isModel, providerStateKey } from '../state.js';
import { MODELS_SCOPE, type Model } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::reconcile',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = typeof obj.provider === 'string' ? obj.provider : '';
      if (!provider) {
        throw new Error('models::reconcile requires a provider');
      }
      const raw = Array.isArray(obj.models) ? obj.models : [];
      const models: Model[] = [];
      for (const entry of raw) {
        if (!isModel(entry)) continue;
        if (entry.provider !== provider) {
          throw new Error(
            `models::reconcile: model ${entry.id} has provider ${entry.provider}, expected ${provider}`,
          );
        }
        models.push(entry);
      }
      await stateSet(iii, MODELS_SCOPE, providerStateKey(provider), models);
      return { provider, count: models.length, ids: models.map((m) => m.id) };
    },
    {
      description:
        'Replace the catalog for <provider> with a single Model[] value under scope models (one state write).',
    },
  );
}
