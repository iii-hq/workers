/**
 * State-backed CRUD. Mirrors `llm-budget/src/store.rs`.
 */

import type { ISdk } from '../runtime/iii.js';
import {
  type Budget,
  SCOPE,
  type SpendLogEntry,
  budgetKey,
  resetLogKey,
  spendLogKey,
} from './types.js';

async function stateSet(iii: ISdk, key: string, value: unknown): Promise<void> {
  await iii.trigger<unknown, unknown>({
    function_id: 'state::set',
    payload: { scope: SCOPE, key, value },
  });
}

async function stateGetValue(iii: ISdk, key: string): Promise<unknown | null> {
  const resp = await iii.trigger<unknown, unknown>({
    function_id: 'state::get',
    payload: { scope: SCOPE, key },
  });
  if (resp === null || resp === undefined) return null;
  if (resp && typeof resp === 'object' && 'value' in (resp as Record<string, unknown>)) {
    const v = (resp as Record<string, unknown>).value;
    return v === null || v === undefined ? null : v;
  }
  return resp;
}

async function stateList(iii: ISdk, prefix: string): Promise<unknown[]> {
  const resp = await iii.trigger<unknown, unknown>({
    function_id: 'state::list',
    payload: { scope: SCOPE, prefix },
  });
  if (Array.isArray(resp)) return resp;
  if (resp && typeof resp === 'object') {
    const items = (resp as Record<string, unknown>).items;
    if (Array.isArray(items)) return items;
  }
  return [];
}

async function stateDelete(iii: ISdk, key: string): Promise<void> {
  await iii.trigger<unknown, unknown>({
    function_id: 'state::delete',
    payload: { scope: SCOPE, key },
  });
}

export async function loadBudget(iii: ISdk, id: string): Promise<Budget | null> {
  const v = await stateGetValue(iii, budgetKey(id));
  if (v === null) return null;
  return v as Budget;
}

export async function saveBudget(iii: ISdk, b: Budget): Promise<void> {
  await stateSet(iii, budgetKey(b.id), b);
}

export async function deleteBudgetRecord(iii: ISdk, id: string): Promise<void> {
  await stateDelete(iii, budgetKey(id));
}

export async function listAllBudgets(iii: ISdk): Promise<Budget[]> {
  const items = await stateList(iii, 'budget:');
  const out: Budget[] = [];
  for (const v of items) {
    const inner =
      v && typeof v === 'object' && 'value' in (v as Record<string, unknown>)
        ? (v as Record<string, unknown>).value
        : v;
    if (inner && typeof inner === 'object' && (inner as Budget).id) out.push(inner as Budget);
  }
  return out;
}

export async function saveSpendLog(
  iii: ISdk,
  id: string,
  period_start: number,
  e: SpendLogEntry,
): Promise<void> {
  await stateSet(iii, spendLogKey(id, period_start), e);
}

export async function saveResetLog(
  iii: ISdk,
  id: string,
  period_start: number,
  ts: number,
  suffix: string,
  e: SpendLogEntry,
): Promise<void> {
  await stateSet(iii, resetLogKey(id, period_start, ts, suffix), e);
}

export async function listSpendLogs(iii: ISdk, budget_id: string): Promise<SpendLogEntry[]> {
  const items = await stateList(iii, `spend_log:${budget_id}:`);
  const out: SpendLogEntry[] = [];
  for (const v of items) {
    const inner =
      v && typeof v === 'object' && 'value' in (v as Record<string, unknown>)
        ? (v as Record<string, unknown>).value
        : v;
    if (inner && typeof inner === 'object' && (inner as SpendLogEntry).budget_id === budget_id) {
      out.push(inner as SpendLogEntry);
    }
  }
  return out;
}
