import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { budgetsEqualForPersist, maybeRollOver, pruneExemptions, withBudgetLock } from '../ops.js';
import { loadBudget, saveBudget } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::check',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      const est = typeof obj.estimated_cost_usd === 'number' ? obj.estimated_cost_usd : 0;
      if (!Number.isFinite(est) || est < 0) {
        throw new Error('estimated_cost_usd must be a finite number >= 0');
      }
      return await withBudgetLock(id, async () => {
        const now_ms = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const rolled = await maybeRollOver(iii, loaded, now_ms);
        const b = pruneExemptions(rolled, now_ms);
        if (!budgetsEqualForPersist(loaded, b)) {
          await saveBudget(iii, b);
        }
        const remaining = b.ceiling_usd - b.spent_usd;
        if (b.paused) return { allowed: true, remaining_usd: remaining, reason: 'paused' };
        if (!b.enforced) return { allowed: true, remaining_usd: remaining, reason: 'not_enforced' };
        const pid = typeof obj.principal_id === 'string' ? obj.principal_id : undefined;
        if (pid && b.exemptions.some((e) => e.principal_id === pid)) {
          return { allowed: true, remaining_usd: remaining, reason: 'exempt' };
        }
        if (remaining < est) {
          return { allowed: false, remaining_usd: remaining, reason: 'ceiling_exceeded' };
        }
        return { allowed: true, remaining_usd: remaining };
      });
    },
    { description: 'Check whether a budget allows an estimated spend.' },
  );
}
