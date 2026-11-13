// The persisted "follow turns" toggle for the TRACES tab.
//
// The masthead's follow toggle (auto-open the trace of the active session's
// live turn) lives in the host's settings (`TracesSettingsStore`) under
// `traces.followTurns`, next to the saved views. Absent means ON —
// following is the out-of-the-box experience; the settings record an
// explicit opt-out. Toggles hit local state immediately; persistence is a
// best-effort read-modify-write of the whole settings value, so an
// unreachable store never lags the toggle — the choice then simply lives
// in memory for the session. Cross-tab concurrency is last-write-wins,
// same as saved views.

import { useMutation } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import { useStoredSettings } from './useStoredSettings'

/** Parse `traces.followTurns` out of the raw settings value.
 *  `undefined` = no choice recorded (callers default to ON). */
function parseFollowTurns(
  configValue: Record<string, unknown>,
): boolean | undefined {
  const traces = configValue.traces
  if (!traces || typeof traces !== 'object') return undefined
  const on = (traces as Record<string, unknown>).followTurns
  return typeof on === 'boolean' ? on : undefined
}

/** Write the toggle back into a (copied) settings value. */
function withFollowTurns(
  configValue: Record<string, unknown>,
  on: boolean,
): Record<string, unknown> {
  const traces =
    configValue.traces && typeof configValue.traces === 'object'
      ? { ...(configValue.traces as Record<string, unknown>) }
      : {}
  traces.followTurns = on
  return { ...configValue, traces }
}

export interface UseFollowTurnsReturn {
  followTurns: boolean
  toggleFollowTurns: () => void
}

export function useFollowTurns(): UseFollowTurnsReturn {
  const { settings, data } = useStoredSettings()

  // Choice made in THIS tab; once set it wins over the server value so a
  // slow write never lags the toggle. `undefined` = no local choice yet.
  const [chosen, setChosen] = useState<boolean | undefined>(undefined)
  const stored = data ? parseFollowTurns(data) : undefined
  const followTurns = chosen ?? stored ?? true

  const mutation = useMutation({
    mutationFn: async (on: boolean) => {
      if (!settings) throw new Error('no settings store')
      const next = settings.update(
        (value) => withFollowTurns(value, on),
        data ?? {},
      )
      await settings.settled()
      return next
    },
  })

  const toggleFollowTurns = useCallback(() => {
    const next = !followTurns
    setChosen(next)
    // Best-effort server persist; the in-memory choice stays live even when
    // the settings store is unreachable.
    mutation.mutateAsync(next).catch(() => {})
  }, [followTurns, mutation])

  return { followTurns, toggleFollowTurns }
}
