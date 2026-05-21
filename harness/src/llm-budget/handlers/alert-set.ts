import { randomUUID } from 'node:crypto';
import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { withBudgetLock } from '../ops.js';
import { loadBudget, saveBudget } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::alert_set',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      return await withBudgetLock(id, async () => {
        const threshold_pct =
          typeof obj.threshold_pct === 'number' ? obj.threshold_pct : Number.NaN;
        if (!Number.isFinite(threshold_pct)) {
          throw new Error('threshold_pct must be a finite number');
        }
        if (threshold_pct <= 0 || threshold_pct > 1) {
          throw new Error('threshold_pct must be in (0, 1]');
        }
        const callback_function_id = requireString(obj, 'callback_function_id');
        const callback_payload = obj.callback_payload;
        const b = await loadBudget(iii, id);
        if (!b) throw new Error(`budget not found: ${id}`);
        const alert_id = randomUUID();
        const next = {
          ...b,
          alerts: [
            ...b.alerts,
            {
              alert_id,
              threshold_pct,
              callback_function_id,
              callback_payload,
              last_fired_period_start: undefined,
            },
          ],
          updated_at: Date.now(),
        };
        await saveBudget(iii, next);
        return { alert_id };
      });
    },
    { description: 'Add an alert to a budget.' },
  );
}
