/**
 * UTC-anchored period math. Mirrors `llm-budget/src/periods.rs`.
 */

import type { Period } from './types.js';

const MS_PER_DAY = 24 * 60 * 60 * 1000;

export function utcDayStart(ms: number): number {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate(), 0, 0, 0, 0);
}

export function utcWeekStart(ms: number): number {
  const dayStart = utcDayStart(ms);
  const dow = (new Date(dayStart).getUTCDay() + 6) % 7; // Monday = 0
  return dayStart - dow * MS_PER_DAY;
}

export function utcMonthStart(ms: number): number {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), 1, 0, 0, 0, 0);
}

export function periodStart(p: Period, ms: number): number {
  if (p === 'day') return utcDayStart(ms);
  if (p === 'week') return utcWeekStart(ms);
  return utcMonthStart(ms);
}

export function nextPeriodStart(p: Period, start: number): number {
  if (p === 'day') return start + MS_PER_DAY;
  if (p === 'week') return start + 7 * MS_PER_DAY;
  // month
  const d = new Date(start);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1, 0, 0, 0, 0);
}

export function periodKey(p: Period, start: number): string {
  const d = new Date(start);
  const y = d.getUTCFullYear();
  const m = (d.getUTCMonth() + 1).toString().padStart(2, '0');
  const day = d.getUTCDate().toString().padStart(2, '0');
  if (p === 'day') return `${y}-${m}-${day}`;
  if (p === 'month') return `${y}-${m}`;
  // ISO week
  const target = new Date(Date.UTC(y, d.getUTCMonth(), d.getUTCDate()));
  const dayOfWeek = (target.getUTCDay() + 6) % 7;
  target.setUTCDate(target.getUTCDate() - dayOfWeek + 3);
  const firstThursday = new Date(Date.UTC(target.getUTCFullYear(), 0, 4));
  const diff = (target.getTime() - firstThursday.getTime()) / MS_PER_DAY;
  const week = 1 + Math.round((diff - ((firstThursday.getUTCDay() + 6) % 7) + 3) / 7);
  return `${target.getUTCFullYear()}-W${String(week).padStart(2, '0')}`;
}

export function daysElapsed(start: number, now: number): number {
  const v = (now - start) / MS_PER_DAY;
  return v < 1 ? 1 : v;
}

export function daysRemaining(now: number, resetsAt: number): number {
  const v = (resetsAt - now) / MS_PER_DAY;
  return v < 0 ? 0 : v;
}
