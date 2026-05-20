import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { withBudgetLock } from '../ops.js';
import { loadBudget, saveBudget } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::enforce',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      const enforced = typeof obj.enforced === 'boolean' ? obj.enforced : null;
      if (enforced === null) throw new Error('enforced must be a boolean');
      return await withBudgetLock(id, async () => {
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const next = { ...loaded, enforced, updated_at: Date.now() };
        await saveBudget(iii, next);
        return { budget_id: next.id, enforced: next.enforced };
      });
    },
    { description: 'Toggle enforcement on a budget.' },
  );
}
