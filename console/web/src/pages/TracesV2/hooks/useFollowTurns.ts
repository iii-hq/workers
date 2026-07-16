// Server-persisted "follow turns" toggle for the TRACES tab.
//
// The masthead's follow toggle (auto-open the trace of the active chat's
// live turn) lives in the engine's `console` configuration entry under
// `traces.followTurns`, next to the saved views. Absent means ON —
// following is the out-of-the-box experience; the config records an
// explicit opt-out. Toggles hit local state immediately; persistence is a
// best-effort read-modify-write of the whole entry (`configuration::set`
// has no partial-update surface), so an unreachable configuration worker
// never lags the toggle — the choice then simply lives in memory for the
// session. Cross-tab concurrency is last-write-wins, same as saved views.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import {
  type ConsoleConfigValue,
  fetchConsoleConfigValue,
  setConsoleConfigValue,
} from '@/lib/console-config'

// Same key as `useTraceViews` / `useSpanFilterSelection` — all three hooks
// read the one `console` entry, sharing the React Query cache.
const CONSOLE_CONFIG_QUERY_KEY = ['consoleConfig']

/** Parse `traces.followTurns` out of the raw console-config value.
 *  `undefined` = no choice recorded (callers default to ON). */
function parseFollowTurns(
  configValue: Record<string, unknown>,
): boolean | undefined {
  const traces = configValue.traces
  if (!traces || typeof traces !== 'object') return undefined
  const on = (traces as Record<string, unknown>).followTurns
  return typeof on === 'boolean' ? on : undefined
}

/** Write the toggle back into a (copied) console-config value. */
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
  const qc = useQueryClient()

  const { data } = useQuery<ConsoleConfigValue | null>({
    queryKey: CONSOLE_CONFIG_QUERY_KEY,
    queryFn: fetchConsoleConfigValue,
    staleTime: 30_000,
    retry: 1,
  })

  // Choice made in THIS tab; once set it wins over the server value so a
  // slow write never lags the toggle. `undefined` = no local choice yet.
  const [chosen, setChosen] = useState<boolean | undefined>(undefined)
  const stored = data ? parseFollowTurns(data) : undefined
  const followTurns = chosen ?? stored ?? true

  const mutation = useMutation({
    mutationFn: async (on: boolean) => {
      const current = (await fetchConsoleConfigValue()) ?? {}
      const next = withFollowTurns(current, on)
      await setConsoleConfigValue(next)
      return next
    },
    onSuccess: (next) => {
      qc.setQueryData(CONSOLE_CONFIG_QUERY_KEY, next)
    },
  })

  const toggleFollowTurns = useCallback(() => {
    const next = !followTurns
    setChosen(next)
    // Best-effort server persist; the in-memory choice stays live even when
    // the configuration worker is unreachable.
    mutation.mutateAsync(next).catch(() => {})
  }, [followTurns, mutation])

  return { followTurns, toggleFollowTurns }
}
