import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { pruneExemptions, withBudgetLock } from '../ops.js';
import { loadBudget, saveBudget } from '../store.js';
import type { Exemption } from '../types.js';

const EXEMPT_TTL_MS = 24 * 60 * 60 * 1000;

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::exempt',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      const principal_id = requireString(obj, 'principal_id');
      const reason = requireString(obj, 'reason');
      return await withBudgetLock(id, async () => {
        const now_ms = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const pruned = pruneExemptions(loaded, now_ms);
        const without = pruned.exemptions.filter((e) => e.principal_id !== principal_id);
        const exemption: Exemption = {
          principal_id,
          reason,
          expires_at: now_ms + EXEMPT_TTL_MS,
        };
        const next = {
          ...pruned,
          exemptions: [...without, exemption],
          updated_at: now_ms,
        };
        await saveBudget(iii, next);
        return { budget_id: next.id, expires_at: exemption.expires_at };
      });
    },
    { description: 'Add a 24h exemption for a principal.' },
  );
}
