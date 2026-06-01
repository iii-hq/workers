import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { getFromState } from '../state.js';
import { parseCapability, supportsModel } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::supports',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = requireString(obj, 'provider');
      const model_id = requireString(obj, 'model_id');
      const capability = parseCapability(requireString(obj, 'capability'));
      if (!capability) {
        throw new Error('missing or unknown capability');
      }
      const m = await getFromState(iii, provider, model_id);
      return { supported: m ? supportsModel(m, capability) : false };
    },
    {
      description:
        'Check whether a provider-registered model supports a capability (false when unknown).',
    },
  );
}
