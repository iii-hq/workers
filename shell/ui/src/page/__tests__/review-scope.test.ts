import { describe, expect, it } from 'vitest'
import {
  DEFAULT_REVIEW_SCOPE,
  EMPTY_TURN_FALLBACK_MS,
  isLiveGitReviewScope,
  isShellUiStatePath,
  shouldFallbackToTurnScope,
  shouldFollowHarnessTurn,
} from '../review-scope'

describe('review scope defaults', () => {
  it('opens cumulative uncommitted changes by default', () => {
    expect(DEFAULT_REVIEW_SCOPE).toEqual({ kind: 'uncommitted' })
  })

  it('refreshes live Git scopes without replacing historical selections', () => {
    expect(isLiveGitReviewScope({ kind: 'uncommitted' })).toBe(true)
    expect(isLiveGitReviewScope({ kind: 'unstaged' })).toBe(true)
    expect(isLiveGitReviewScope({ kind: 'staged' })).toBe(true)
    expect(isLiveGitReviewScope({ kind: 'last-turn' })).toBe(false)
    expect(isLiveGitReviewScope({ kind: 'turn', turnId: 't1', label: 'Turn 1' })).toBe(false)
    expect(
      isLiveGitReviewScope({
        kind: 'commit',
        sha: 'abc123',
        subject: 'previous work',
      }),
    ).toBe(false)
  })

  it('follows a new Harness turn until the user selects another scope', () => {
    expect(shouldFollowHarnessTurn(true, DEFAULT_REVIEW_SCOPE)).toBe(true)
    expect(shouldFollowHarnessTurn(false, DEFAULT_REVIEW_SCOPE)).toBe(false)
    expect(shouldFollowHarnessTurn(false, { kind: 'last-turn' })).toBe(true)
    expect(
      shouldFollowHarnessTurn(false, {
        kind: 'commit',
        sha: 'abc123',
        subject: 'previous work',
      }),
    ).toBe(false)
  })

  it('ignores Shell UI persistence events that would refresh the active diff again', () => {
    expect(isShellUiStatePath('config/shell-ui.yaml')).toBe(true)
    expect(isShellUiStatePath('config/shell-ui.yaml.tmp')).toBe(true)
    expect(isShellUiStatePath('project/config/shell-ui.yaml')).toBe(true)
    expect(isShellUiStatePath('project\\config\\shell-ui.yaml.tmp')).toBe(true)
    expect(isShellUiStatePath('docs/shell-ui.yaml')).toBe(false)
    expect(isShellUiStatePath('src/app.ts')).toBe(false)
  })

  it('uses turn entries when a live Git scope is opened outside a repository', () => {
    expect(shouldFallbackToTurnScope(DEFAULT_REVIEW_SCOPE, 'not-a-repo')).toBe(true)
    expect(shouldFallbackToTurnScope({ kind: 'staged' }, 'ready')).toBe(false)
    expect(shouldFallbackToTurnScope({ kind: 'last-turn' }, 'not-a-repo')).toBe(false)
  })

  it('leaves enough time for the final workspace event batch before empty fallback', () => {
    expect(EMPTY_TURN_FALLBACK_MS).toBeGreaterThan(650)
  })
})
