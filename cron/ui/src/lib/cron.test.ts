import { describe, expect, it } from 'vitest'
import { describeCron, nextCronRun, validateCron } from './cron'

describe('describeCron', () => {
  it('describes common seconds-first schedules', () => {
    expect(describeCron('0 0 17 21 7 *')).toBe('At 17:00 on July 21')
    expect(describeCron('0 30 9 * * *')).toBe('Every day at 09:30')
    expect(describeCron('0 */5 * * * *')).toBe('Every 5 minutes')
    expect(describeCron('*/5 * * * * *')).toBe('Every 5 seconds')
    expect(describeCron('* * * * * *')).toBe('Every second')
    expect(describeCron('30 * * * * *')).toBe('Every minute at second 30')
    expect(describeCron('0 15 * * * *')).toBe('At minute 15 of every hour')
    expect(describeCron('0 0 */6 * * *')).toBe('Every 6 hours at 00:00')
  })

  it('describes Rust cron weekday numbering and names', () => {
    expect(describeCron('0 0 9 * * 1')).toBe('Every Sunday at 09:00')
    expect(describeCron('0 0 9 * * 2,6')).toBe('Every Monday and Friday at 09:00')
    expect(describeCron('0 0 9 * * Mon,Fri')).toBe('Every Monday and Friday at 09:00')
    expect(describeCron('0 0 0 1 * *')).toBe('At 00:00 on day 1 of every month')
    expect(describeCron('0 0 12 * Jul *')).toBe('At 12:00 in July')
  })

  it('refuses ambiguous or unsupported translations', () => {
    expect(describeCron('0 0 17 21 7 1')).toBeNull()
    expect(describeCron('0 0-30 9 * * *')).toBeNull()
    expect(describeCron('0 0 9 L * *')).toBeNull()
    expect(describeCron('0 0 25 * * *')).toBeNull()
    expect(describeCron('0 0 17 21 7 * 2027')).toBeNull()
    expect(describeCron('0 0 9 * * 0')).toBeNull()
    expect(describeCron('*/7 * * * * *')).toBeNull()
    expect(describeCron('0 */7 * * * *')).toBeNull()
    expect(describeCron('0 0 */5 * * *')).toBeNull()
    expect(describeCron('30 9 * * *')).toBeNull()
    expect(describeCron('nonsense')).toBeNull()
  })

  it('keeps valid non-uniform steps schedulable without mislabelling them', () => {
    const expression = '0 */7 * * * *'
    expect(validateCron(expression)).toBeNull()
    expect(describeCron(expression)).toBeNull()
    expect(nextCronRun(expression, new Date('2026-08-20T12:56:00Z'))).toEqual(new Date('2026-08-20T13:00:00Z'))
  })
})
