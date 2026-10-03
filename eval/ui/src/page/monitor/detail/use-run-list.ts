// The E2E runs the attach dialog offers, read once when it opens. The form
// mounts with the dialog, so "once" is once per opening; Try again reads again.
import { useCallback, useEffect, useRef, useState } from 'react'
import type { EvalApi } from '../../../api'
import { type E2eRun, e2eRuns } from './e2e-runs'

export type RunList = { phase: 'loading' } | { phase: 'ready'; runs: E2eRun[] } | { phase: 'failed' }

export function useRunList(api: EvalApi): { list: RunList; reload: () => void; fail: () => void } {
  const [list, setList] = useState<RunList>({ phase: 'loading' })
  const latest = useRef(0)
  const alive = useRef(true)
  // Set again on every mount: StrictMode mounts, unmounts and mounts a dev tree.
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])

  const reload = useCallback(() => {
    const request = ++latest.current
    const settle = (next: RunList) => alive.current && request === latest.current && setList(next)
    setList({ phase: 'loading' })
    api
      .e2eExecutions()
      .then((response) => settle({ phase: 'ready', runs: e2eRuns(response) }))
      .catch(() => settle({ phase: 'failed' }))
  }, [api])

  const fail = useCallback(() => setList({ phase: 'failed' }), [])

  useEffect(reload, [reload])
  return { list, reload, fail }
}
