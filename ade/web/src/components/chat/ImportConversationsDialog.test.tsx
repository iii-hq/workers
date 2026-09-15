import { expect, it } from 'vitest'
import { summaryText } from './ImportConversationsDialog'

it('announces a single imported conversation', () => {
  expect(
    summaryText({
      imported: 1,
      messages: 67,
      failed: 0,
      lastSessionId: 'session-a',
    }),
  ).toBe('Imported 1 conversation · 67 messages')
})

it('announces a batch', () => {
  expect(
    summaryText({
      imported: 2,
      messages: 70,
      failed: 0,
      lastSessionId: 'session-b',
    }),
  ).toBe('Imported 2 conversations · 70 messages')
})

it('keeps a partial failure visible beside what was imported', () => {
  expect(
    summaryText({
      imported: 1,
      messages: 1,
      failed: 1,
      lastSessionId: 'session-c',
    }),
  ).toBe('Imported 1 conversation · 1 message · 1 failed')
})

it('does not claim an import when every conversation failed', () => {
  expect(
    summaryText({ imported: 0, messages: 0, failed: 2, lastSessionId: null }),
  ).toBe('Nothing imported · 2 failed')
})
