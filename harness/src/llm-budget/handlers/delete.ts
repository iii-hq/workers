import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { withBudgetLock } from '../ops.js';
import { deleteBudgetRecord } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::delete',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      return await withBudgetLock(id, async () => {
        await deleteBudgetRecord(iii, id);
        return { ok: true };
      });
    },
    { description: 'Delete a budget.' },
  );
}
