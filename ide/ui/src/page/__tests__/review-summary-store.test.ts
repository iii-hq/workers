import { describe, expect, it, vi } from 'vitest'
import {
  clearShellReviewSummary,
  emitShellReviewFileSelection,
  getShellReviewSummary,
  publishShellReviewSummary,
  subscribeShellReviewFileSelection,
  subscribeShellReviewSummary,
} from '../review-summary-state'

describe('Shell review summary store', () => {
  it('publishes a session-scoped snapshot without retaining file bodies', () => {
    const sessionId = 'summary-normalizes'
    const listener = vi.fn()
    const off = subscribeShellReviewSummary(sessionId, listener)
    const files = [
      {
        path: 'src/app.ts',
        state: 'ready' as const,
        add: 12,
        del: 3,
        oldContents: 'large old body',
        newContents: 'large new body',
      },
    ]

    publishShellReviewSummary(sessionId, {
      sourceId: 'tab-a',
      turnId: 'turn-1',
      files,
    })

    expect(getShellReviewSummary(sessionId)).toEqual({
      sourceId: 'tab-a',
      turnId: 'turn-1',
      files: [{ path: 'src/app.ts', state: 'ready', add: 12, del: 3 }],
    })
    expect(listener).toHaveBeenCalledTimes(1)

    off()
    clearShellReviewSummary(sessionId, 'tab-a')
  })

  it('falls back to the previous explorer when the latest publisher leaves', () => {
    const sessionId = 'summary-fallback'
    publishShellReviewSummary(sessionId, {
      sourceId: 'tab-a',
      turnId: 'turn-a',
      files: [{ path: 'a.ts', state: 'ready', add: 1, del: 0 }],
    })
    publishShellReviewSummary(sessionId, {
      sourceId: 'tab-b',
      turnId: 'turn-b',
      files: [{ path: 'b.ts', state: 'ready', add: 2, del: 1 }],
    })

    expect(getShellReviewSummary(sessionId)?.sourceId).toBe('tab-b')
    clearShellReviewSummary(sessionId, 'tab-b')
    expect(getShellReviewSummary(sessionId)?.sourceId).toBe('tab-a')

    clearShellReviewSummary(sessionId, 'tab-a')
    expect(getShellReviewSummary(sessionId)).toBeNull()
  })

  it('preserves pending totals instead of normalizing them to zero', () => {
    const sessionId = 'summary-pending'
    publishShellReviewSummary(sessionId, {
      sourceId: 'tab-a',
      turnId: 'turn-a',
      files: [
        {
          path: 'large.ts',
          state: 'pending',
          add: null,
          del: null,
        },
      ],
    })

    expect(getShellReviewSummary(sessionId)?.files).toEqual([
      {
        path: 'large.ts',
        state: 'pending',
        add: null,
        del: null,
      },
    ])
    clearShellReviewSummary(sessionId, 'tab-a')
  })

  it('emits targeted file selections to Shell subscribers', () => {
    const sessionId = 'summary-selection'
    const listener = vi.fn()
    const off = subscribeShellReviewFileSelection(sessionId, listener)

    emitShellReviewFileSelection(sessionId, {
      sourceId: 'tab-a',
      path: 'src/app.ts',
    })

    expect(listener).toHaveBeenCalledWith({
      sourceId: 'tab-a',
      path: 'src/app.ts',
    })
    off()
  })
})
