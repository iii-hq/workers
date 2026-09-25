import { describe, expect, it } from 'vitest'
import {
  addActiveTime,
  dismiss,
  emptyRecord,
  isEmailish,
  PROMPT_AFTER_MS,
  parseRecord,
  SNOOZE_MS,
  shouldPrompt,
  snooze,
  subscribed,
} from './email-prompt'

const TICK = 15_000

describe('earning a prompt', () => {
  it('waits for thirty minutes of active time', () => {
    let record = emptyRecord()
    for (let banked = 0; banked < PROMPT_AFTER_MS; banked += TICK) {
      expect(shouldPrompt(record, 0)).toBe(false)
      record = addActiveTime(record, TICK, TICK)
    }
    expect(record.activeMs).toBe(PROMPT_AFTER_MS)
    expect(shouldPrompt(record, 0)).toBe(true)
  })

  it('counts a sleeping tab as one tick, not the gap it slept for', () => {
    // A laptop shut overnight must not wake up owing the whole night.
    const record = addActiveTime(emptyRecord(), 9 * 60 * 60 * 1000, TICK)
    expect(record.activeMs).toBe(TICK)
    expect(shouldPrompt(record, 0)).toBe(false)
  })

  it('banks nothing once a decision is made', () => {
    for (const record of [dismiss(emptyRecord()), subscribed(emptyRecord())]) {
      const later = addActiveTime(record, TICK, TICK)
      expect(later.activeMs).toBe(0)
      expect(shouldPrompt(later, Date.now())).toBe(false)
    }
  })
})

describe('asking again', () => {
  const earned = { ...emptyRecord(), activeMs: PROMPT_AFTER_MS }

  it('holds a snoozed prompt for at least 36 hours', () => {
    const now = 1_000_000
    const snoozed = snooze(earned, now)

    expect(SNOOZE_MS).toBe(36 * 60 * 60 * 1000)
    expect(shouldPrompt(snoozed, now)).toBe(false)
    expect(shouldPrompt(snoozed, now + SNOOZE_MS - 1)).toBe(false)
    expect(shouldPrompt(snoozed, now + SNOOZE_MS)).toBe(true)
  })

  it('never asks again after a decision', () => {
    const now = Date.now()
    expect(shouldPrompt(dismiss(earned), now + SNOOZE_MS * 10)).toBe(false)
    expect(shouldPrompt(subscribed(earned), now + SNOOZE_MS * 10)).toBe(false)
  })
})

describe('the stored record', () => {
  it('survives a round trip', () => {
    const record = snooze({ ...emptyRecord(), activeMs: 42 }, 1_000)
    expect(parseRecord(JSON.stringify(record))).toEqual(record)
  })

  it('falls back to a fresh record on anything unexpected', () => {
    for (const raw of [
      null,
      '',
      'not json',
      '[]',
      '"pending"',
      '{"status":"whatever","activeMs":"soon","snoozeUntil":null}',
      '{"activeMs":-5,"snoozeUntil":-5}',
    ]) {
      expect(parseRecord(raw)).toEqual(emptyRecord())
    }
  })
})

describe('the address', () => {
  it('accepts one plain address', () => {
    expect(isEmailish('someone@example.com')).toBe(true)
    expect(isEmailish('  someone@example.co.uk  ')).toBe(true)
    expect(isEmailish('SOMEONE@EXAMPLE.COM')).toBe(true)
    // A local part may carry dots, an apostrophe, a plus tag and a hyphen.
    expect(isEmailish("first.o'last+tag-1@sub.example.com")).toBe(true)
    expect(isEmailish('a@b-c.io')).toBe(true)
  })

  it('refuses anything that is not one address', () => {
    for (const value of [
      '',
      '   ',
      'not-an-address',
      '@example.com',
      'someone@example',
      'one@a.co, two@b.co',
      'Someone <someone@a.co>',
      // A dot may not lead, repeat, or end the local part.
      '.someone@example.com',
      'some..one@example.com',
      'someone.@example.com',
      // The domain needs an alphabetic TLD and no leading hyphen.
      'someone@example.c0m',
      'someone@-example.com',
      'someone@example..com',
      `${'a'.repeat(250)}@example.com`,
    ]) {
      expect(isEmailish(value), value).toBe(false)
    }
  })
})
