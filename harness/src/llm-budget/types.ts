export type Period = 'day' | 'week' | 'month';

export type Alert = {
  alert_id: string;
  threshold_pct: number;
  callback_function_id: string;
  callback_payload?: unknown;
  last_fired_period_start?: number;
};

export type Exemption = {
  principal_id: string;
  reason: string;
  expires_at: number;
};

export type Budget = {
  id: string;
  workspace_id?: string;
  agent_id?: string;
  name?: string;
  ceiling_usd: number;
  period: Period;
  spent_usd: number;
  period_start_at: number;
  period_resets_at: number;
  enforced: boolean;
  paused: boolean;
  alerts: Alert[];
  exemptions: Exemption[];
  created_at: number;
  updated_at: number;
};

export type SpendLogEntry = {
  budget_id: string;
  period_start: number;
  period_end: number;
  spent_usd: number;
  records_count: number;
};

export const SCOPE = 'budgets';

export function budgetKey(id: string): string {
  return `budget:${id}`;
}

export function spendLogKey(id: string, period_start: number): string {
  return `spend_log:${id}:${period_start}`;
}

export function resetLogKey(id: string, period_start: number, ts: number, suffix: string): string {
  return `spend_log:${id}:${period_start}:reset-${ts}-${suffix}`;
}
