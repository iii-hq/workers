import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { budgetsEqualForPersist, maybeRollOver, withBudgetLock } from '../ops.js';
import { daysElapsed, daysRemaining } from '../periods.js';
import { loadBudget, saveBudget } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::forecast',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      return await withBudgetLock(id, async () => {
        const now_ms = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const b = await maybeRollOver(iii, loaded, now_ms);
        if (!budgetsEqualForPersist(loaded, b)) await saveBudget(iii, b);
        const elapsed = daysElapsed(b.period_start_at, now_ms);
        const rate = b.spent_usd / elapsed;
        const projected_month = rate * 30;
        const remaining_budget = b.ceiling_usd - b.spent_usd;
        const days_until_breach = rate > 0 && remaining_budget > 0 ? remaining_budget / rate : null;
        const remaining_days = daysRemaining(now_ms, b.period_resets_at);
        const on_track = rate * remaining_days <= remaining_budget;
        return { projected_month_usd: projected_month, on_track, days_until_breach };
      });
    },
    { description: 'Project spend through period end.' },
  );
}
