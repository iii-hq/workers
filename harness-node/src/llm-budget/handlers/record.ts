import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { logger } from '../../runtime/otel.js';
import { maybeRollOver, withBudgetLock } from '../ops.js';
import { loadBudget, saveBudget } from '../store.js';
import type { Alert } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::record',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const id = requireString(obj, 'budget_id');
      const cost = typeof obj.cost_usd === 'number' ? obj.cost_usd : Number.NaN;
      if (!Number.isFinite(cost) || cost < 0) throw new Error('cost_usd must be >= 0');
      return await withBudgetLock(id, async () => {
        const now_ms = Date.now();
        const loaded = await loadBudget(iii, id);
        if (!loaded) throw new Error(`budget not found: ${id}`);
        const b = await maybeRollOver(iii, loaded, now_ms);
        b.spent_usd += cost;
        b.updated_at = now_ms;
        const ratio = b.ceiling_usd > 0 ? b.spent_usd / b.ceiling_usd : 0;
        const pending: Alert[] = [];
        for (const a of b.alerts) {
          if (ratio >= a.threshold_pct && a.last_fired_period_start !== b.period_start_at) {
            pending.push({ ...a });
            a.last_fired_period_start = b.period_start_at;
          }
        }
        await saveBudget(iii, b);
        for (const a of pending) {
          const cb_payload: Record<string, unknown> = {
            ...(a.callback_payload &&
            typeof a.callback_payload === 'object' &&
            !Array.isArray(a.callback_payload)
              ? (a.callback_payload as Record<string, unknown>)
              : {}),
            alert_id: a.alert_id,
            budget_id: b.id,
            spent_usd: b.spent_usd,
            ceiling_usd: b.ceiling_usd,
            threshold_pct: a.threshold_pct,
          };
          // Fire-and-forget alert callback. Mirrors the Rust `tokio::spawn`.
          iii
            .trigger<unknown, unknown>({
              function_id: a.callback_function_id,
              payload: cb_payload,
            })
            .catch((err) => {
              logger.warn('budget alert callback failed', {
                callback: a.callback_function_id,
                err: String(err),
              });
            });
        }
        return { spent_usd: b.spent_usd, remaining_usd: b.ceiling_usd - b.spent_usd };
      });
    },
    { description: 'Record a spend, fire matching alerts.' },
  );
}
