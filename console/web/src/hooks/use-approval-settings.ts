import { useCallback, useEffect, useRef, useState } from 'react'
import {
  addAlwaysAllow as rpcAddAlwaysAllow,
  type ApprovalSettings,
  approveAlways as rpcApproveAlways,
  DEFAULT_APPROVAL_SETTINGS,
  getApprovalSettings,
  type PermissionMode,
  removeAlwaysAllow as rpcRemoveAlwaysAllow,
  setApprovalMode,
} from '@/lib/backend/approval-settings'
import { loadDefaultAllowlist, loadDefaultPermissionMode } from '@/lib/storage'

interface UseApprovalSettingsResult {
  settings: ApprovalSettings
  loaded: boolean
  setMode(mode: PermissionMode): Promise<void>
  addAlwaysAllow(functionId: string): Promise<void>
  removeAlwaysAllow(functionId: string): Promise<void>
  approveAlways(functionId: string): Promise<void>
}

/**
 * Per-conversation approval settings hook. Loads on mount and on session
 * change; mutators write through to the backend and refresh local state.
 *
 * Each mutator returns a Promise so callers can chain UI feedback. Errors
 * are swallowed and logged in dev — the user-facing fallback is the prompt
 * still appearing (Manual mode default).
 */
export function useApprovalSettings(
  sessionId: string,
): UseApprovalSettingsResult {
  const [settings, setSettings] = useState<ApprovalSettings>(
    DEFAULT_APPROVAL_SETTINGS,
  )
  const [loaded, setLoaded] = useState(false)
  const activeRef = useRef(sessionId)

  useEffect(() => {
    activeRef.current = sessionId
    setLoaded(false)
    let cancelled = false
    void (async () => {
      try {
        const fetched = await getApprovalSettings(sessionId)
        if (cancelled || activeRef.current !== sessionId) return
        // First-touch initialization: when the backend has no persisted
        // settings for this session (mode_set_at === 0), seed both the
        // user-level default mode AND the user-level allowlist from
        // localStorage. Applies to NEW conversations only — existing
        // sessions return a non-zero mode_set_at and skip this branch
        // entirely so the user's later edits don't retroactively touch
        // conversations already in flight.
        const isFirstTouch = fetched.mode_set_at === 0
        if (isFirstTouch) {
          const userDefault = loadDefaultPermissionMode()
          const defaultAllowlist = loadDefaultAllowlist()
          let next = fetched
          if (userDefault !== 'manual') {
            next = await setApprovalMode(sessionId, userDefault)
            if (cancelled || activeRef.current !== sessionId) return
          }
          for (const functionId of defaultAllowlist) {
            next = await rpcAddAlwaysAllow(sessionId, functionId)
            if (cancelled || activeRef.current !== sessionId) return
          }
          setSettings(next)
        } else {
          setSettings(fetched)
        }
        setLoaded(true)
      } catch (err) {
        if (cancelled) return
        console.error(
          '[approval-settings] load failed — is approval-gate running with the new handlers? mode picker will default to "manual" and writes will fail.',
          err,
        )
        setSettings(DEFAULT_APPROVAL_SETTINGS)
        setLoaded(true)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [sessionId])

  const setMode = useCallback(
    async (mode: PermissionMode) => {
      // Optimistic — flip the UI now so the click feels responsive even
      // when the backend round-trip is slow. Revert on failure.
      let previous: ApprovalSettings | null = null
      setSettings((s) => {
        previous = s
        return { ...s, mode, mode_set_at: Date.now() }
      })
      try {
        const next = await setApprovalMode(sessionId, mode)
        if (activeRef.current === sessionId) setSettings(next)
      } catch (err) {
        console.error('[approval-settings] set_mode failed', err)
        if (activeRef.current === sessionId && previous) {
          setSettings(previous)
        }
      }
    },
    [sessionId],
  )

  const addAlwaysAllow = useCallback(
    async (functionId: string) => {
      let previous: ApprovalSettings | null = null
      setSettings((s) => {
        previous = s
        if (s.always_allow.some((e) => e.function_id === functionId)) return s
        return {
          ...s,
          always_allow: [
            ...s.always_allow,
            {
              function_id: functionId,
              granted_at: Date.now(),
              granted_by: 'user_click',
            },
          ],
        }
      })
      try {
        const next = await rpcAddAlwaysAllow(sessionId, functionId)
        if (activeRef.current === sessionId) setSettings(next)
      } catch (err) {
        console.error('[approval-settings] add_always_allow failed', err)
        if (activeRef.current === sessionId && previous) {
          setSettings(previous)
        }
      }
    },
    [sessionId],
  )

  const removeAlwaysAllow = useCallback(
    async (functionId: string) => {
      let previous: ApprovalSettings | null = null
      setSettings((s) => {
        previous = s
        return {
          ...s,
          always_allow: s.always_allow.filter(
            (e) => e.function_id !== functionId,
          ),
        }
      })
      try {
        const next = await rpcRemoveAlwaysAllow(sessionId, functionId)
        if (activeRef.current === sessionId) setSettings(next)
      } catch (err) {
        console.error('[approval-settings] remove_always_allow failed', err)
        if (activeRef.current === sessionId && previous) {
          setSettings(previous)
        }
      }
    },
    [sessionId],
  )

  const approveAlways = useCallback(
    async (functionId: string) => {
      let previous: ApprovalSettings | null = null
      setSettings((s) => {
        previous = s
        if (s.approved_always.some((e) => e.function_id === functionId))
          return s
        return {
          ...s,
          approved_always: [
            ...s.approved_always,
            {
              function_id: functionId,
              granted_at: Date.now(),
              granted_by: 'user_click',
            },
          ],
        }
      })
      try {
        const next = await rpcApproveAlways(sessionId, functionId)
        if (activeRef.current === sessionId) setSettings(next)
      } catch (err) {
        console.error('[approval-settings] approve_always failed', err)
        if (activeRef.current === sessionId && previous) {
          setSettings(previous)
        }
      }
    },
    [sessionId],
  )

  return {
    settings,
    loaded,
    setMode,
    addAlwaysAllow,
    removeAlwaysAllow,
    approveAlways,
  }
}
