/**
 * State-backed CRUD. Mirrors `llm-budget/src/store.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import { createState } from '../runtime/state.js';
import {
  type Budget,
  SCOPE,
  type SpendLogEntry,
  budgetKey,
  resetLogKey,
  spendLogKey,
} from './types.js';

function strictState(iii: ISdk) {
  return createState(iii, { tolerant: false });
}

function isBudget(v: unknown): v is Budget {
  return (
    v !== null &&
    typeof v === 'object' &&
    typeof (v as Budget).id === 'string' &&
    typeof (v as Budget).ceiling_usd === 'number'
  );
}

function isSpendLogEntry(v: unknown, budget_id: string): v is SpendLogEntry {
  return (
    v !== null &&
    typeof v === 'object' &&
    (v as SpendLogEntry).budget_id === budget_id &&
    !('ceiling_usd' in (v as Record<string, unknown>))
  );
}

export async function loadBudget(iii: ISdk, id: string): Promise<Budget | null> {
  const v = await strictState(iii).get<Budget>({ scope: SCOPE, key: budgetKey(id) });
  if (v === null) return null;
  return isBudget(v) ? v : null;
}

export async function saveBudget(iii: ISdk, b: Budget): Promise<void> {
  await strictState(iii).set({ scope: SCOPE, key: budgetKey(b.id), value: b });
}

export async function deleteBudgetRecord(iii: ISdk, id: string): Promise<void> {
  await strictState(iii).delete({ scope: SCOPE, key: budgetKey(id) });
}

export async function listAllBudgets(iii: ISdk): Promise<Budget[]> {
  const items = await strictState(iii).list<unknown>({ scope: SCOPE });
  return items.filter(isBudget);
}

export async function saveSpendLog(
  iii: ISdk,
  id: string,
  period_start: number,
  e: SpendLogEntry,
): Promise<void> {
  await strictState(iii).set({ scope: SCOPE, key: spendLogKey(id, period_start), value: e });
}

export async function saveResetLog(
  iii: ISdk,
  id: string,
  period_start: number,
  ts: number,
  suffix: string,
  e: SpendLogEntry,
): Promise<void> {
  await strictState(iii).set({
    scope: SCOPE,
    key: resetLogKey(id, period_start, ts, suffix),
    value: e,
  });
}

export async function listSpendLogs(iii: ISdk, budget_id: string): Promise<SpendLogEntry[]> {
  const items = await strictState(iii).list<unknown>({ scope: SCOPE });
  return items.filter((v): v is SpendLogEntry => isSpendLogEntry(v, budget_id));
}
