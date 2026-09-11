import { releaseWorktree } from '@/lib/worktrees'

/**
 * Session-scoped registry of worktree claims made through THIS console's
 * picker flow. Release-on-switch must never release a claim some other
 * surface (or another agent) took, so only worktrees recorded here are ever
 * auto-released when a conversation closes or its working directory moves
 * away. In-memory on purpose: a reload drops auto-release, never a claim.
 */

export interface ConsoleWorktreeClaim {
  worktreeId: string
  /** The worktree path the conversation's workingDir was set to. */
  path: string
}

const claims = new Map<string, ConsoleWorktreeClaim>()

export function recordConsoleClaim(
  sessionId: string,
  claim: ConsoleWorktreeClaim,
): void {
  claims.set(sessionId, claim)
}

export function consoleClaimFor(
  sessionId: string,
): ConsoleWorktreeClaim | null {
  return claims.get(sessionId) ?? null
}

export function forgetConsoleClaim(sessionId: string): void {
  claims.delete(sessionId)
}

/**
 * Pure decision: release when a claim exists and the conversation is not
 * still pointed at that worktree's path.
 */
export function shouldReleaseClaim(
  claim: ConsoleWorktreeClaim | null,
  keepPath: string | null,
): claim is ConsoleWorktreeClaim {
  if (!claim) return false
  return keepPath === null || keepPath !== claim.path
}

type ReleaseFn = (worktreeId: string, sessionId: string) => Promise<void>

/**
 * Release the session's console-made claim, if any, unless the conversation
 * still points at the claimed worktree (`keepPath`). Best-effort: a failed
 * release is logged and forgotten — the worktree worker's prune sweep is the
 * durable backstop for stale claims.
 */
export async function releaseConsoleClaimIfAny(
  sessionId: string,
  opts: { keepPath?: string | null; release?: ReleaseFn } = {},
): Promise<void> {
  const claim = consoleClaimFor(sessionId)
  if (!shouldReleaseClaim(claim, opts.keepPath ?? null)) return
  claims.delete(sessionId)
  const release = opts.release ?? releaseWorktree
  try {
    await release(claim.worktreeId, sessionId)
  } catch (err) {
    if (import.meta.env.DEV) {
      console.warn('[worktrees] release failed', claim.worktreeId, err)
    }
  }
}

export function __resetWorktreeClaimsForTests(): void {
  claims.clear()
}
