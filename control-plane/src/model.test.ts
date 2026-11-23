import { describe, expect, it } from 'vitest'
import {
  isInScope,
  isStalled,
  linkedCard,
  needUrgency,
  prWaitReason,
  sortNeeds,
} from './model.js'

describe('needs-you ordering', () => {
  it('derives urgency and orders by urgency, blocked cards, then age', () => {
    const items = [
      {
        kind: 'question' as const,
        questionClass: 'routine' as const,
        blockedCards: 0,
        createdAt: 1,
      },
      {
        kind: 'question' as const,
        questionClass: 'scope' as const,
        blockedCards: 2,
        createdAt: 3,
      },
      { kind: 'stalled' as const, blockedCards: 2, createdAt: 2 },
      {
        kind: 'question' as const,
        questionClass: 'security' as const,
        blockedCards: 0,
        createdAt: 0,
      },
    ]

    expect(needUrgency(items[0])).toBe('no-rush')
    expect(sortNeeds(items)).toEqual([
      items[2],
      items[1],
      items[3],
      items[0],
    ])
  })
})

describe('stalled runs', () => {
  const now = Date.parse('2026-11-22T12:00:00Z')
  const stalled = {
    status: 'done',
    runningChildren: 0,
    openQuestions: 0,
    expectsWake: true,
    armedWakes: [{ expires_at: null }],
    lastProgress: now - 31 * 60_000,
  }

  it('requires an overdue indefinite wake and no work or questions', () => {
    expect(isStalled(stalled, now)).toBe(true)
    expect(isStalled({ ...stalled, runningChildren: 1 }, now)).toBe(false)
    expect(isStalled({ ...stalled, openQuestions: 1 }, now)).toBe(false)
    expect(isStalled({ ...stalled, armedWakes: [] }, now)).toBe(true)
  })

  it('does not stall a wait with a future deadline', () => {
    expect(
      isStalled({ ...stalled, armedWakes: [{ expires_at: now + 1 }] }, now),
    ).toBe(false)
  })
})

describe('pull requests', () => {
  const cards = [{ key: 'KAN-12' }, { key: 'KAN-123' }]

  it('links a card key from the title or head branch without prefix matches', () => {
    expect(linkedCard('Ship KAN-12 safely', 'feature', cards)?.key).toBe(
      'KAN-12',
    )
    expect(linkedCard('Ship', 'feat/KAN-123-api', cards)?.key).toBe('KAN-123')
    expect(linkedCard('Ship KAN-1234', 'feature', cards)).toBeUndefined()
  })

  it('uses the deterministic wait reason priority', () => {
    expect(
      prWaitReason({
        isDraft: true,
        checks: 'failing',
        reviewDecision: 'REVIEW_REQUIRED',
        mergeable: 'CONFLICTING',
      }),
    ).toBe('Draft')
    expect(prWaitReason({ checks: 'failing' })).toBe('CI failing')
    expect(
      prWaitReason({ checks: 'passing', reviewDecision: 'REVIEW_REQUIRED' }),
    ).toBe('Review requested')
    expect(prWaitReason({ checks: 'passing', mergeStateStatus: 'DIRTY' })).toBe(
      'Merge conflict',
    )
  })
})

describe('project scope', () => {
  it('keeps every row for all projects and only exact project rows otherwise', () => {
    expect(isInScope('all', 'a')).toBe(true)
    expect(isInScope('all', undefined)).toBe(true)
    expect(isInScope('a', 'a')).toBe(true)
    expect(isInScope('a', 'b')).toBe(false)
    expect(isInScope('a', undefined)).toBe(false)
  })
})
