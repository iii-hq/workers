import type { ISdk } from '../../runtime/iii.js';
import { stateSet } from '../../runtime/state.js';
import { modelKey } from '../state.js';
import { MODELS_SCOPE, type Model } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::register',
    async (payload: unknown) => {
      const m = payload as Model;
      if (!m || typeof m !== 'object' || !m.provider || !m.id) {
        throw new Error('invalid Model payload: provider and id are required');
      }
      const key = modelKey(m.provider, m.id);
      await stateSet(iii, MODELS_SCOPE, key, m);
      return { key, registered: true };
    },
    { description: 'Write a model to iii state under models:<provider>:<id>.' },
  );
}
