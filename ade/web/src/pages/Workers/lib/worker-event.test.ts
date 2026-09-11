import { describe, expect, it } from 'vitest'
import { parseWorkerEvent, workerEventSchema } from './worker-event'

describe('workerEventSchema', () => {
  it('parses a terminal worker lifecycle event', () => {
    const payload = {
      operation: 'stop',
      stage: 'done',
      worker: 'iii-directory',
      timestamp_ms: 1_700_000_000_000,
      caller_mode: 'cli',
    }
    expect(workerEventSchema.parse(payload)).toEqual(payload)
    expect(parseWorkerEvent(payload)).toEqual(payload)
  })

  it('parses a failed event with structured error', () => {
    const payload = {
      operation: 'add',
      stage: 'failed',
      worker: 'harness',
      timestamp_ms: 1_700_000_000_001,
      caller_mode: 'trigger',
      error: { code: 'W900', message: 'download failed' },
    }
    expect(parseWorkerEvent(payload)?.error?.message).toBe('download failed')
  })

  it('rejects incomplete payloads', () => {
    expect(parseWorkerEvent({ operation: 'add' })).toBeNull()
  })
})
