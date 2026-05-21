import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { loadBudget } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::get',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      const b = await loadBudget(iii, id);
      if (!b) throw new Error(`budget not found: ${id}`);
      return { budget: b };
    },
    { description: 'Fetch a budget by id.' },
  );
}
