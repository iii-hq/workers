import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { getFromState } from '../state.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::get',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      const model_id = requireString(obj, 'model_id');
      return await getFromState(iii, provider, model_id);
    },
    {
      description:
        'Look up a single model by (provider, model_id). Returns null when no provider has registered it.',
    },
  );
}
