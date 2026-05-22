import { randomUUID } from 'node:crypto';
import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { maybeRollOver, withBudgetLock } from '../ops.js';
import { nextPeriodStart, periodStart } from '../periods.js';
import { loadBudget, saveBudget, saveResetLog } from '../store.js';
import type { Budget } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::reset',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      return await withBudgetLock(id, async () => {
        const ts = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const b = await maybeRollOver(iii, loaded, ts);
        const previous = b.spent_usd;
        const start = periodStart(b.period, ts);
        const reset: Budget = {
          ...b,
          spent_usd: 0,
          period_start_at: start,
          period_resets_at: nextPeriodStart(b.period, start),
          updated_at: ts,
          alerts: b.alerts.map((a) => ({ ...a, last_fired_period_start: undefined })),
        };
        await saveBudget(iii, reset);
        const suffix = randomUUID();
        try {
          await saveResetLog(iii, b.id, b.period_start_at, ts, suffix, {
            budget_id: b.id,
            period_start: b.period_start_at,
            period_end: ts,
            spent_usd: previous,
            records_count: 0,
          });
        } catch (e) {
          logger.error('reset archive save failed after budget reset committed', {
            budget_id: b.id,
            previous,
            err: String(e),
          });
        }
        return { budget_id: b.id, previous_spent_usd: previous };
      });
    },
    { description: 'Reset spent_usd, archive prior period.' },
  );
}
