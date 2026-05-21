import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { maybeRollOver, withBudgetLock } from '../ops.js';
import { nextPeriodStart, periodStart } from '../periods.js';
import { loadBudget, saveBudget, saveSpendLog } from '../store.js';
import type { Budget, Period } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::update',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      if (!obj.patch || typeof obj.patch !== 'object') {
        throw new Error('patch must be an object');
      }
      const patch = obj.patch as Record<string, unknown>;
      return await withBudgetLock(id, async () => {
        const ts = Date.now();
        const current = await loadBudget(iii, id);
        if (!current) throw new Error(`budget not found: ${id}`);

        let new_ceiling: number | undefined;
        if (patch.ceiling_usd !== undefined) {
          const c = patch.ceiling_usd;
          if (typeof c !== 'number' || !Number.isFinite(c) || c <= 0) {
            throw new Error('ceiling_usd must be a positive finite number');
          }
          new_ceiling = c;
        }

        let new_period: Period | undefined;
        if (patch.period !== undefined) {
          if (typeof patch.period !== 'string') throw new Error('period must be a string');
          if (patch.period !== 'day' && patch.period !== 'week' && patch.period !== 'month') {
            throw new Error(`invalid period: ${patch.period}`);
          }
          new_period = patch.period;
        }

        let new_enforced: boolean | undefined;
        if (patch.enforced !== undefined) {
          if (typeof patch.enforced !== 'boolean') throw new Error('enforced must be a boolean');
          new_enforced = patch.enforced;
        }

        let new_paused: boolean | undefined;
        if (patch.paused !== undefined) {
          if (typeof patch.paused !== 'boolean') throw new Error('paused must be a boolean');
          new_paused = patch.paused;
        }

        const new_name = typeof patch.name === 'string' ? patch.name : undefined;

        const period_changing = new_period !== undefined && new_period !== current.period;
        const rolled = period_changing ? await maybeRollOver(iii, current, ts) : current;

        const next: Budget = {
          ...rolled,
          ceiling_usd: new_ceiling ?? rolled.ceiling_usd,
          period: new_period ?? rolled.period,
          enforced: new_enforced ?? rolled.enforced,
          paused: new_paused ?? rolled.paused,
          name: new_name ?? rolled.name,
          updated_at: ts,
        };

        if (period_changing) {
          await saveSpendLog(iii, rolled.id, rolled.period_start_at, {
            budget_id: rolled.id,
            period_start: rolled.period_start_at,
            period_end: ts,
            spent_usd: rolled.spent_usd,
            records_count: 0,
          });
          next.period_start_at = periodStart(next.period, ts);
          next.period_resets_at = nextPeriodStart(next.period, next.period_start_at);
          next.spent_usd = 0;
          next.alerts = next.alerts.map((a) => ({
            ...a,
            last_fired_period_start: undefined,
          }));
        }

        await saveBudget(iii, next);
        return { budget: next };
      });
    },
    { description: 'Update a whitelisted set of budget fields.' },
  );
}
