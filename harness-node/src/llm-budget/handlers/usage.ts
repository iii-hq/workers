import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { budgetsEqualForPersist, maybeRollOver, withBudgetLock } from '../ops.js';
import { periodStart } from '../periods.js';
import { listSpendLogs, loadBudget, saveBudget } from '../store.js';
import type { Period } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::usage',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      return await withBudgetLock(id, async () => {
        const ts = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const b = await maybeRollOver(iii, loaded, ts);
        if (!budgetsEqualForPersist(loaded, b)) await saveBudget(iii, b);

        const window = typeof obj.window === 'string' ? obj.window : 'all';
        if (!['all', 'day', 'week', 'month'].includes(window)) {
          throw new Error(`invalid window: ${window}`);
        }
        if (window !== 'all' && window !== b.period) {
          throw new Error(
            `window '${window}' does not align with budget period '${b.period}'. Use window: '${b.period}' or 'all'.`,
          );
        }
        const cutoff = window === 'all' ? 0 : periodStart(window as Period, ts);

        const logs = await listSpendLogs(iii, id);
        const relevant = logs.filter(
          (l) => l.period_start >= cutoff && l.period_start !== b.period_start_at,
        );
        const by_period = relevant
          .map((l) => ({ period: l.period_start, spent: l.spent_usd }))
          .sort((a, c) => a.period - c.period);
        if (b.period_start_at >= cutoff) {
          by_period.push({ period: b.period_start_at, spent: b.spent_usd });
        }
        const spent = by_period.reduce((s, e) => s + e.spent, 0);
        return { spent_usd: spent, by_period, records_count: by_period.length };
      });
    },
    { description: 'Aggregate spend over a window.' },
  );
}
