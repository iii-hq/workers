import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, EmptyState, type Host } from '@iii-dev/console-ui'
import { type Subscribe, useStateEvents } from '../lib/events'
import { LiveDot, useFlash } from '../lib/widgets'

export function ScopesView({
  host,
  subscribe,
  onOpen,
}: {
  host: Host
  subscribe: Subscribe
  onOpen: (scope: string) => void
}) {
  const [scopes, setScopes] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [flashed, flash] = useFlash()

  const load = useCallback(() => {
    host.iii
      .trigger<{ groups: string[] }>('state::list_groups', {})
      .then((r) => {
        setError(null)
        setScopes(r.groups)
      })
      .catch((err: unknown) =>
        setError(err instanceof Error ? err.message : String(err)),
      )
  }, [host])
  useEffect(load, [load])

  // Deletes can empty a scope — only the worker knows, so refetch
  // (debounced across bursts).
  const refetchTimer = useRef<number | null>(null)
  const scheduleRefetch = useCallback(() => {
    if (refetchTimer.current !== null) window.clearTimeout(refetchTimer.current)
    refetchTimer.current = window.setTimeout(() => {
      refetchTimer.current = null
      load()
    }, 300)
  }, [load])
  useEffect(
    () => () => {
      if (refetchTimer.current !== null)
        window.clearTimeout(refetchTimer.current)
    },
    [],
  )

  useStateEvents(subscribe, (e) => {
    if (e.event_type === 'state:deleted') {
      scheduleRefetch()
      return
    }
    // created/updated: a write into an unseen scope = a new scope, live.
    setScopes((prev) => {
      if (!prev || prev.includes(e.scope)) return prev
      flash(e.scope)
      return [...prev, e.scope].sort()
    })
  })

  return (
    <>
      <div className="state-ui-head">
        <span className="state-ui-title">state scopes</span>
        <LiveDot />
        <span style={{ flex: 1 }} />
        <Button variant="pill" size="sm" onClick={load}>
          refresh
        </Button>
      </div>
      {error ? (
        <div className="state-ui-error">state::list_groups failed — {error}</div>
      ) : scopes === null ? (
        <div className="state-ui-list">
          <div className="state-ui-note">loading scopes…</div>
        </div>
      ) : scopes.length === 0 ? (
        <EmptyState
          title="no state scopes yet"
          description="nothing has written to state — a scope appears here live with its first state::set."
        />
      ) : (
        <div className="state-ui-list">
          {scopes.map((scope) => (
            <button
              key={scope}
              type="button"
              className={`state-ui-row${flashed.has(scope) ? ' flash' : ''}`}
              onClick={() => onOpen(scope)}
            >
              <span>{scope}</span>
              <span className="arrow">▸</span>
            </button>
          ))}
        </div>
      )}
    </>
  )
}
