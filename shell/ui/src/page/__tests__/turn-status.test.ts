import { describe, expect, it } from 'vitest'
import {
  activeTurnFromStatus,
  canCaptureHarnessWorkspaceChange,
  canActivateHarnessTurn,
  HARNESS_CHANGE_DRAIN_MS,
  isActiveHarnessStatus,
} from '../turn-status'

describe('isActiveHarnessStatus', () => {
  it('recognizes only in-flight Harness states', () => {
    expect(isActiveHarnessStatus('running')).toBe(true)
    expect(isActiveHarnessStatus('awaiting_functions')).toBe(true)
    expect(isActiveHarnessStatus('completed')).toBe(false)
    expect(isActiveHarnessStatus(undefined)).toBe(false)
  })
})

describe('activeTurnFromStatus', () => {
  it('restores an in-flight turn when no lifecycle event overtook the request', () => {
    expect(
      activeTurnFromStatus(
        { session_id: 'session-1', turn_id: 'turn-1', status: 'running' },
        'session-1',
        4,
        4,
        new Set(),
      ),
    ).toBe('turn-1')
  })

  it('rejects a stale running response after a completion was observed', () => {
    expect(
      activeTurnFromStatus(
        { session_id: 'session-1', turn_id: 'turn-1', status: 'running' },
        'session-1',
        4,
        5,
        new Set(['turn-1']),
      ),
    ).toBeNull()
  })

  it('rejects inactive and cross-session snapshots', () => {
    expect(
      activeTurnFromStatus(
        { session_id: 'session-1', turn_id: 'turn-1', status: 'completed' },
        'session-1',
        2,
        2,
        new Set(),
      ),
    ).toBeNull()
    expect(
      activeTurnFromStatus(
        { session_id: 'session-2', turn_id: 'turn-1', status: 'running' },
        'session-1',
        2,
        2,
        new Set(),
      ),
    ).toBeNull()
  })

  it('rejects a stale status for a turn completed before the request began', () => {
    expect(
      activeTurnFromStatus(
        { session_id: 'session-1', turn_id: 'turn-1', status: 'running' },
        'session-1',
        7,
        7,
        new Set(['turn-1']),
      ),
    ).toBeNull()
  })
})

describe('canActivateHarnessTurn', () => {
  it('keeps terminal completion sticky against a late duplicate start', () => {
    const completed = new Set(['turn-1'])

    expect(canActivateHarnessTurn('turn-1', completed)).toBe(false)
    expect(canActivateHarnessTurn('turn-2', completed)).toBe(true)
  })
})

describe('canCaptureHarnessWorkspaceChange', () => {
  const completed = {
    turnId: 'turn-1',
    epoch: 3,
    active: false,
    completedAtMs: 1_000,
  }

  it('accepts the final buffered batch during the bounded completion drain', () => {
    expect(
      canCaptureHarnessWorkspaceChange(
        completed,
        'turn-1',
        3,
        1_000 + HARNESS_CHANGE_DRAIN_MS,
      ),
    ).toBe(true)
  })

  it('rejects later writes after the completion drain closes', () => {
    expect(
      canCaptureHarnessWorkspaceChange(
        completed,
        'turn-1',
        3,
        1_001 + HARNESS_CHANGE_DRAIN_MS,
      ),
    ).toBe(false)
  })

  it('invalidates the drain immediately for a different turn or epoch', () => {
    expect(canCaptureHarnessWorkspaceChange(completed, 'turn-2', 3, 1_001)).toBe(false)
    expect(canCaptureHarnessWorkspaceChange(completed, 'turn-1', 4, 1_001)).toBe(false)
  })

  it('accepts an active pre-turn window without a completion deadline', () => {
    expect(
      canCaptureHarnessWorkspaceChange(
        { ...completed, active: true, completedAtMs: null },
        'turn-1',
        3,
        99_000,
      ),
    ).toBe(true)
  })
})
