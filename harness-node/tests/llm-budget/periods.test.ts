import { describe, expect, it } from 'vitest';
import {
  daysElapsed,
  daysRemaining,
  nextPeriodStart,
  periodKey,
  utcDayStart,
  utcMonthStart,
  utcWeekStart,
} from '../../src/llm-budget/periods.js';

const ms = (y: number, m: number, d: number, h = 0) => Date.UTC(y, m - 1, d, h, 0, 0, 0);

describe('periods', () => {
  it('utcDayStart zeros time', () => {
    const t = ms(2026, 4, 30, 17);
    expect(new Date(utcDayStart(t)).getUTCHours()).toBe(0);
    expect(new Date(utcDayStart(t)).getUTCDate()).toBe(30);
  });

  it('utcWeekStart anchors on Monday', () => {
    const t = ms(2026, 4, 30, 17);
    const ws = utcWeekStart(t);
    const date = new Date(ws);
    expect(date.getUTCDate()).toBe(27);
    expect((date.getUTCDay() + 6) % 7).toBe(0);
  });

  it('utcMonthStart returns first of month at midnight', () => {
    const t = ms(2026, 4, 30, 17);
    const s = utcMonthStart(t);
    expect(new Date(s).getUTCDate()).toBe(1);
    expect(new Date(s).getUTCMonth()).toBe(3);
  });

  it('nextPeriodStart handles year wrap for month', () => {
    const dec = ms(2026, 12, 1);
    const jan = nextPeriodStart('month', dec);
    expect(new Date(jan).getUTCFullYear()).toBe(2027);
    expect(new Date(jan).getUTCMonth()).toBe(0);
  });

  it('periodKey day format', () => {
    const s = utcDayStart(ms(2026, 4, 30, 17));
    expect(periodKey('day', s)).toBe('2026-04-30');
  });

  it('periodKey month format', () => {
    const s = utcMonthStart(ms(2026, 4, 30, 17));
    expect(periodKey('month', s)).toBe('2026-04');
  });

  it('periodKey week starts with year-W', () => {
    const s = utcWeekStart(ms(2026, 4, 30, 17));
    expect(periodKey('week', s)).toMatch(/^2026-W\d{2}$/);
  });

  it('daysElapsed floors at 1', () => {
    const now = ms(2026, 4, 30, 1);
    expect(daysElapsed(now, now)).toBeCloseTo(1);
  });

  it('daysRemaining clamps at 0', () => {
    const now = ms(2026, 4, 30, 1);
    expect(daysRemaining(now, now - 24 * 60 * 60 * 1000)).toBe(0);
  });
});
