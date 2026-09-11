import { useEffect, useState } from 'react'
import {
  getWorktree,
  validateWorktreePath,
  type WorktreeInfo,
} from '@/lib/worktrees'

/**
 * Resolve a conversation's working directory to the managed worktree behind
 * it, if any: `worktree::validate` on the path, then `worktree::get` for the
 * branch/lifecycle/status the badge renders. Returns `null` while resolving,
 * when the path is not a managed worktree, or when the probe fails — the
 * caller falls back to the plain path chip. `refreshToken` re-runs the
 * resolve (bumped on landed / land-blocked events).
 */
export function useWorktreeBinding(
  workingDir: string | null,
  enabled: boolean,
  refreshToken: number,
): WorktreeInfo | null {
  const [info, setInfo] = useState<WorktreeInfo | null>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshToken is a re-run token (bumped on landed / land-blocked events), not read by the effect body
  useEffect(() => {
    if (!enabled || !workingDir) {
      setInfo(null)
      return
    }
    let cancelled = false
    void (async () => {
      try {
        const validated = await validateWorktreePath(workingDir)
        if (cancelled) return
        if (!validated?.managed || !validated.worktree_id) {
          setInfo(null)
          return
        }
        const next = await getWorktree(validated.worktree_id)
        if (!cancelled) setInfo(next)
      } catch {
        if (!cancelled) setInfo(null)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [enabled, workingDir, refreshToken])

  return info
}
