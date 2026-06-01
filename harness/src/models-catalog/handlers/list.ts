import type { ISdk } from '../../runtime/iii.js';
import { listFromState } from '../state.js';
import { parseCapability } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::list',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = typeof obj.provider === 'string' ? obj.provider : undefined;
      const cap = typeof obj.capability === 'string' ? parseCapability(obj.capability) : null;
      const models = await listFromState(iii, {
        provider,
        capability: cap ?? undefined,
      });
      return { models };
    },
    {
      description:
        'List models, optionally filtered by provider or capability. Returns only models registered by providers (no embedded seed).',
    },
  );
}
