import { useCallback, useEffect, useState } from 'react'
import { Button, EmptyState, type Host } from '@iii-dev/console-ui'
import { type Subscribe, useStateEvents } from '../lib/events'
import { BackButton, LiveDot, useFlash } from '../lib/widgets'

export function ItemsView({
  host,
  scope,
  subscribe,
  onBack,
  onOpen,
}: {
  host: Host
  scope: string
  subscribe: Subscribe
  onBack: () => void
  onOpen: (key: string) => void
}) {
  const [keys, setKeys] = useState<string[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [flashed, flash] = useFlash()

  const load = useCallback(() => {
    host.iii
      .trigger<{ keys: string[] }>('state::list_keys', { scope })
      .then((r) => {
        setError(null)
        setKeys(r.keys)
      })
      .catch((err: unknown) =>
        setError(err instanceof Error ? err.message : String(err)),
      )
  }, [host, scope])
  useEffect(load, [load])

  useStateEvents(subscribe, (e) => {
    if (e.scope !== scope) return
    if (e.event_type === 'state:deleted') {
      setKeys((prev) => prev?.filter((k) => k !== e.key) ?? prev)
      return
    }
    flash(e.key)
    // created: append (kv scopes are insertion-ordered); updated: no
    // membership change, just the flash above.
    setKeys((prev) => {
      if (!prev || prev.includes(e.key)) return prev
      return [...prev, e.key]
    })
  })

  return (
    <>
      <div className="state-ui-head">
        <BackButton onClick={onBack} label="back to scopes" />
        <span className="state-ui-crumb">{scope}</span>
        <LiveDot />
        <span style={{ flex: 1 }} />
        <Button variant="pill" size="sm" onClick={load}>
          refresh
        </Button>
      </div>
      {error ? (
        <div className="state-ui-error">state::list_keys failed — {error}</div>
      ) : keys === null ? (
        <div className="state-ui-list">
          <div className="state-ui-note">loading items…</div>
        </div>
      ) : keys.length === 0 ? (
        <EmptyState
          title="scope is empty"
          description={`no keys under '${scope}' — new writes appear here live; the scope disappears once its last key is deleted.`}
        />
      ) : (
        <div className="state-ui-list">
          {keys.map((key) => (
            <button
              key={key}
              type="button"
              className={`state-ui-row${flashed.has(key) ? ' flash' : ''}`}
              onClick={() => onOpen(key)}
            >
              <span>{key}</span>
              <span className="arrow">▸</span>
            </button>
          ))}
        </div>
      )}
    </>
  )
}
