import { useCallback, useRef, useState } from 'react'
import {
  listGrantedDirs,
  revokeDir as rpcRevokeDir,
} from '@/lib/backend/filesystem-grants'

interface UseFilesystemGrantsResult {
  /** Session-scoped granted dirs (plain path strings — no metadata). */
  grants: string[]
  /** False once a `harness::filesystem::grants` call has failed. */
  supported: boolean
  loaded: boolean
  /** Re-read from the harness. Call when the management dialog opens. */
  refresh(): Promise<void>
  /** Optimistic revoke; reverts on failure. */
  revoke(root: string): Promise<void>
  /**
   * Locally append a freshly-granted dir without a round trip — called right
   * after a filesystem-access prompt resolves with `session`/`always` scope, so
   * the "filesystem access · N" affordance updates immediately instead of
   * waiting for the next dialog open.
   */
  addOptimistic(root: string): void
}

/**
 * Session-scoped filesystem grant management. Loads lazily —
 * callers drive `refresh()` (e.g. on dialog open) rather than the hook
 * auto-fetching on every session change, since the grants group is normally
 * hidden until the user opens the management dialog.
 */
export function useFilesystemGrants(
  sessionId: string,
): UseFilesystemGrantsResult {
  const [grants, setGrants] = useState<string[]>([])
  const [supported, setSupported] = useState(true)
  const [loaded, setLoaded] = useState(false)
  const activeRef = useRef(sessionId)
  activeRef.current = sessionId
  // Monotonic op counter: only the response to the most recently issued
  // read/write may overwrite `grants`, so a slow `refresh()` snapshot can't
  // land after a faster `revoke()` and resurrect the just-revoked folder.
  const seqRef = useRef(0)

  const refresh = useCallback(async () => {
    const forSession = sessionId
    const seq = ++seqRef.current
    const result = await listGrantedDirs(forSession)
    if (activeRef.current !== forSession || seq !== seqRef.current) return
    setGrants(result.dirs)
    setSupported(result.supported)
    setLoaded(true)
  }, [sessionId])

  const revoke = useCallback(
    async (root: string) => {
      const forSession = sessionId
      const seq = ++seqRef.current
      setGrants((g) => g.filter((d) => d !== root))
      try {
        const next = await rpcRevokeDir(forSession, root)
        if (activeRef.current === forSession && seq === seqRef.current) {
          setGrants(next)
        }
      } catch (err) {
        console.error('[filesystem-grants] revoke failed', err)
        // Re-add only the root that failed to revoke, applied against the
        // *current* state — never overwrite with a stale pre-call snapshot,
        // which could clobber a concurrent successful revoke of another root.
        if (activeRef.current === forSession) {
          setGrants((g) => (g.includes(root) ? g : [...g, root]))
        }
      }
    },
    [sessionId],
  )

  const addOptimistic = useCallback((root: string) => {
    // Invalidate any in-flight refresh whose snapshot predates this grant.
    seqRef.current++
    setGrants((g) => (g.includes(root) ? g : [...g, root]))
  }, [])

  return { grants, supported, loaded, refresh, revoke, addOptimistic }
}
