/**
 * Per-budget mutex map and rollover/exemption pruning. Mirrors
 * `llm-budget/src/ops.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { logger } from '../runtime/otel.js';
import { nextPeriodStart } from './periods.js';
import { saveSpendLog } from './store.js';
import type { Budget } from './types.js';

const locks = new Map<string, Promise<void>>();

/** Single-process serializer for a budget's load → mutate → save cycle. */
export async function withBudgetLock<T>(budget_id: string, fn: () => Promise<T>): Promise<T> {
  const prev = locks.get(budget_id) ?? Promise.resolve();
  let release: () => void = () => {};
  const next = new Promise<void>((r) => {
    release = r;
  });
  locks.set(
    budget_id,
    prev.then(() => next),
  );
  try {
    await prev;
    return await fn();
  } finally {
    release();
    if (locks.get(budget_id) === prev.then(() => next)) {
      locks.delete(budget_id);
    }
  }
}

export async function maybeRollOver(iii: ISdk, b: Budget, ts: number): Promise<Budget> {
  if (ts < b.period_resets_at) return b;
  let period_start_at = b.period_start_at;
  let resets_at = b.period_resets_at;
  let spent = b.spent_usd;
  let archived = 0;
  while (ts >= resets_at) {
    await saveSpendLog(iii, b.id, period_start_at, {
      budget_id: b.id,
      period_start: period_start_at,
      period_end: resets_at,
      spent_usd: spent,
      records_count: 0,
    });
    archived++;
    period_start_at = resets_at;
    resets_at = nextPeriodStart(b.period, period_start_at);
    spent = 0;
  }
  if (archived > 0) {
    logger.info('budget rolled over', { budget_id: b.id, archived });
  }
  const alerts = b.alerts.map((a) => ({ ...a, last_fired_period_start: undefined }));
  return {
    ...b,
    period_start_at,
    period_resets_at: resets_at,
    spent_usd: spent,
    alerts,
    updated_at: ts,
  };
}

export function pruneExemptions(b: Budget, ts: number): Budget {
  const live = b.exemptions.filter((e) => e.expires_at > ts);
  if (live.length === b.exemptions.length) return b;
  return { ...b, exemptions: live, updated_at: ts };
}

export function budgetsEqualForPersist(a: Budget, b: Budget): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
