import { randomUUID } from 'node:crypto';
import { requireString } from '../../runtime/handler.js';
import type { ISdk } from '../../runtime/iii.js';
import { nextPeriodStart, periodStart } from '../periods.js';
import { saveBudget } from '../store.js';
import type { Budget, Period } from '../types.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::create',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const ceiling = typeof obj.ceiling_usd === 'number' ? obj.ceiling_usd : Number.NaN;
      if (!Number.isFinite(ceiling) || ceiling <= 0) {
        throw new Error('ceiling_usd must be > 0');
      }
      const period = parsePeriod(requireString(obj, 'period'));
      const now_ms = Date.now();
      const start = periodStart(period, now_ms);
      const b: Budget = {
        id: randomUUID(),
        workspace_id: typeof obj.workspace_id === 'string' ? obj.workspace_id : undefined,
        agent_id: typeof obj.agent_id === 'string' ? obj.agent_id : undefined,
        name: typeof obj.name === 'string' ? obj.name : undefined,
        ceiling_usd: ceiling,
        period,
        spent_usd: 0,
        period_start_at: start,
        period_resets_at: nextPeriodStart(period, start),
        enforced: true,
        paused: false,
        alerts: [],
        exemptions: [],
        created_at: now_ms,
        updated_at: now_ms,
      };
      await saveBudget(iii, b);
      return { budget_id: b.id };
    },
    { description: 'Create a budget with ceiling + period.' },
  );
}

function parsePeriod(s: string): Period {
  if (s === 'day' || s === 'week' || s === 'month') return s;
  throw new Error(`invalid period: ${s} (expected 'day' | 'week' | 'month')`);
}
