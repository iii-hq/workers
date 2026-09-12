import { describe, expect, it } from 'vitest'
import type { StatusChangedEvent } from '@/lib/sessions/types'
import { shouldRingCompletionBell } from './completion-bell'

function statusEvent(
  status: StatusChangedEvent['status'],
  overrides: Partial<StatusChangedEvent> = {},
): StatusChangedEvent {
  return {
    session_id: 'session-1',
    previous_status: 'working',
    status,
    timestamp: 1_000,
    ...overrides,
  }
}

describe('shouldRingCompletionBell', () => {
  it.each(['done', 'error'] as const)(
    'rings for a top-level working → %s transition',
    (status) => {
      expect(
        shouldRingCompletionBell(statusEvent(status), { status: 'working' }),
      ).toBe(true)
    },
  )

  it('does not ring for state hydration or duplicate terminal events', () => {
    expect(
      shouldRingCompletionBell(
        statusEvent('done', { previous_status: 'done' }),
        { status: 'done' },
      ),
    ).toBe(false)
  })

  it('does not ring for known child sessions', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        parentId: 'parent',
        status: 'working',
      }),
    ).toBe(false)
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        depth: 1,
        status: 'working',
      }),
    ).toBe(false)
  })

  it('does not ring for a delayed event older than the current status', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), {
        status: 'working',
        serverStatusUpdatedAt: 2_000,
      }),
    ).toBe(false)
  })

  it('does not ring for a user-stopped turn', () => {
    expect(
      shouldRingCompletionBell(
        statusEvent('done', { status_reason: 'stopped' }),
        { status: 'working' },
      ),
    ).toBe(false)
  })

  it('does not ring after the local conversation is already terminal', () => {
    expect(
      shouldRingCompletionBell(statusEvent('done'), { status: 'done' }),
    ).toBe(false)
  })

  it('does not ring for a session that is not known locally', () => {
    expect(shouldRingCompletionBell(statusEvent('done'), undefined)).toBe(false)
  })
})
