import type { ISdk } from '../../runtime/iii.js';
import { listFromStateOrSeed } from '../state.js';
import { parseCapability } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'models::list',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const provider = typeof obj.provider === 'string' ? obj.provider : undefined;
      const cap = typeof obj.capability === 'string' ? parseCapability(obj.capability) : null;
      const models = await listFromStateOrSeed(iii, {
        provider,
        capability: cap ?? undefined,
      });
      return { models };
    },
    {
      description:
        'List models, optionally filtered by provider or capability. Reads from iii state; falls back to the embedded seed when state is empty.',
    },
  );
}
