import { describe, expect, it } from 'vitest'
import { isPersonQueuedRow } from './queued-strip'

describe('isPersonQueuedRow', () => {
  it('keeps a direct row (a person on this tab or another)', () => {
    expect(isPersonQueuedRow({ channel: 'direct' })).toBe(true)
  })

  it('keeps a row from a harness that predates the channel field', () => {
    expect(isPersonQueuedRow({})).toBe(true)
    expect(isPersonQueuedRow({ channel: undefined })).toBe(true)
  })

  it('hides the harness talking to the agent', () => {
    expect(isPersonQueuedRow({ channel: 'notification' })).toBe(false)
    expect(isPersonQueuedRow({ channel: 'invoked-agent' })).toBe(false)
  })
})
