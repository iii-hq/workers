import type { ReviewScopeSelection } from './ReviewScopePicker'

export type LiveGitReviewScope = Extract<ReviewScopeSelection, { kind: 'uncommitted' | 'unstaged' | 'staged' }>

export const LAST_TURN_SCOPE = {
  kind: 'last-turn',
} as const satisfies ReviewScopeSelection

export const SESSION_SCOPE = {
  kind: 'session',
} as const satisfies ReviewScopeSelection

export const DEFAULT_REVIEW_SCOPE = {
  kind: 'uncommitted',
} as const satisfies LiveGitReviewScope

export const EMPTY_TURN_FALLBACK_MS = 750

export function isLiveGitReviewScope(scope: ReviewScopeSelection): scope is LiveGitReviewScope {
  return scope.kind === 'uncommitted' || scope.kind === 'unstaged' || scope.kind === 'staged'
}

export function isShellUiStatePath(path: string): boolean {
  const normalized = path.replaceAll('\\', '/')
  return /(^|\/)config\/shell-ui\.yaml(?:\.tmp)?$/.test(normalized)
}

export function shouldFollowHarnessTurn(autoFollow: boolean, scope: ReviewScopeSelection): boolean {
  return autoFollow || scope.kind === 'last-turn'
}

export function shouldEnterTurnScope(
  autoFollow: boolean,
  scope: ReviewScopeSelection,
  inRootChanges: number,
): boolean {
  return inRootChanges > 0 && shouldFollowHarnessTurn(autoFollow, scope)
}

export function shouldFallbackToTurnScope(
  scope: ReviewScopeSelection,
  gitState: 'ready' | 'not-a-repo' | 'error',
): boolean {
  return gitState === 'not-a-repo' && isLiveGitReviewScope(scope)
}
