/**
 * The scopes × keys browser the state page is built on.
 *
 * Layout adapts to the width the browser HAS (a ResizeObserver on its own
 * root, not a viewport media query — the console can host it in panes of
 * any size). Wide: three columns — scopes, the selected scope's keys, and
 * the value workspace side by side. Under NARROW_BELOW px it becomes a
 * drill-in flow: scopes → keys → value, one pane at a time with ← back
 * buttons.
 *
 * Live updates ride the page-wide `state` trigger binding
 * (src/lib/events.ts): a write into an unseen scope/key appends to its
 * column with a flash; deletes prune the keys column and refetch the
 * scope list (only the worker knows when a scope emptied — debounced
 * across bursts). The value workspace guards its own draft (ValueEditor
 * reports dirty up); column navigation asks before discarding one.
 */

import { Button, type Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type Subscribe, useStateEvents } from '../lib/events'
import { BackButton, DatabaseIcon, RefreshButton, useFlash } from '../lib/widgets'
import { ValueEditor } from './ValueEditor'

/** Container width (px) below which the browser collapses to the
 * drill-in scopes ⇄ keys ⇄ value flow. */
const NARROW_BELOW = 800

/** Observe the browser root's own width. Returns a callback ref to put
 * on the root plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none) are ignored so a hidden browser keeps its
 * last real layout. */
function useContainerNarrow(threshold: number): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      const width = node.getBoundingClientRect().width
      if (width > 0) setNarrow(width < threshold)
      const observer = new ResizeObserver((entries) => {
        const next = entries[0]?.contentRect.width
        if (typeof next === 'number' && next > 0) setNarrow(next < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

/** Shared column body: error / loading / empty / row list. */
function ColumnBody({
  error,
  noun,
  onRetry,
  rows,
  selected,
  flashed,
  onOpen,
  empty,
}: {
  error: string | null
  noun: string
  onRetry: () => void
  rows: string[] | null
  selected: string | null
  flashed: ReadonlySet<string>
  onOpen: (row: string) => void
  empty: ReactNode
}) {
  return (
    <div className="state-ui-col-scroll">
      {error ? (
        <div className="state-ui-error-panel">
          <p>
            The {noun} list could not be loaded.
            <span className="detail">{error}</span>
          </p>
          <Button variant="ghost" size="sm" onClick={onRetry}>
            retry
          </Button>
        </div>
      ) : rows === null ? (
        <div className="state-ui-col-skel" aria-hidden>
          {[0, 1, 2, 3].map((i) => (
            <span key={i} className={`bar w${[60, 80, 40, 70][i]}`} />
          ))}
        </div>
      ) : rows.length === 0 ? (
        empty
      ) : (
        <ul className="state-ui-nav-list">
          {rows.map((row) => (
            <li key={row}>
              <button
                type="button"
                className={`state-ui-nav-row${row === selected ? ' active' : ''}${flashed.has(row) ? ' flash' : ''}`}
                aria-current={row === selected ? 'true' : undefined}
                onClick={() => onOpen(row)}
              >
                <span className="name">{row}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

export function StateBrowser({
  host,
  subscribe,
  panelSide = 'left',
}: {
  host: Host
  subscribe: Subscribe
  panelSide?: 'left' | 'right'
}) {
  const [scopes, setScopes] = useState<string[] | null>(null)
  const [scopesError, setScopesError] = useState<string | null>(null)
  const [scopeFlash, flashScope] = useFlash()

  const [scope, setScope] = useState<string | null>(null)
  const [key, setKey] = useState<string | null>(null)

  const [keys, setKeys] = useState<string[] | null>(null)
  const [keysError, setKeysError] = useState<string | null>(null)
  const [keyFlash, flashKey] = useFlash()

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  // Live mirrors for the event handler (its closure is kept fresh by
  // useStateEvents, but list membership checks want the latest arrays
  // without re-subscribing per render).
  const scopesRef = useRef(scopes)
  scopesRef.current = scopes
  const keysRef = useRef(keys)
  keysRef.current = keys
  const scopeRef = useRef(scope)
  scopeRef.current = scope

  // The open editor reports unsaved edits up; navigating away asks first.
  const dirtyRef = useRef(false)
  const onDirtyChange = useCallback((dirty: boolean) => {
    dirtyRef.current = dirty
  }, [])
  const confirmDiscard = () => !dirtyRef.current || window.confirm('Discard unsaved changes?')

  const loadScopes = useCallback(() => {
    host.iii
      .trigger<{ groups: string[] }>('state::list_groups', {})
      .then((r) => {
        setScopesError(null)
        setScopes(r.groups)
      })
      .catch((err: unknown) => setScopesError(err instanceof Error ? err.message : String(err)))
  }, [host])
  useEffect(loadScopes, [loadScopes])

  // Deletes can empty a scope — only the worker knows, so refetch
  // (debounced across bursts).
  const refetchTimer = useRef<number | null>(null)
  const scheduleScopesRefetch = useCallback(() => {
    if (refetchTimer.current !== null) window.clearTimeout(refetchTimer.current)
    refetchTimer.current = window.setTimeout(() => {
      refetchTimer.current = null
      loadScopes()
    }, 300)
  }, [loadScopes])
  useEffect(
    () => () => {
      if (refetchTimer.current !== null) window.clearTimeout(refetchTimer.current)
    },
    [],
  )

  const loadKeys = useCallback(() => {
    const current = scopeRef.current
    if (current === null) return
    host.iii
      .trigger<{ keys: string[] }>('state::list_keys', { scope: current })
      .then((r) => {
        // Stale async result: the user switched scopes while this list
        // was in flight — applying it would show A's keys under B.
        if (scopeRef.current !== current) return
        setKeysError(null)
        setKeys(r.keys)
      })
      .catch((err: unknown) => {
        if (scopeRef.current !== current) return
        setKeysError(err instanceof Error ? err.message : String(err))
      })
  }, [host])
  useEffect(() => {
    setKeys(null)
    setKeysError(null)
    if (scope !== null) loadKeys()
  }, [scope, loadKeys])

  useStateEvents(subscribe, (e) => {
    if (e.event_type === 'state:deleted') {
      scheduleScopesRefetch()
      if (e.scope === scopeRef.current) {
        setKeys((prev) => prev?.filter((k) => k !== e.key) ?? prev)
      }
      return
    }
    // created/updated: a write into an unseen scope = a new scope, live.
    if (scopesRef.current && !scopesRef.current.includes(e.scope)) {
      flashScope(e.scope)
      setScopes([...scopesRef.current, e.scope].sort())
    }
    if (e.scope === scopeRef.current) {
      flashKey(e.key)
      // created: append (kv scopes are insertion-ordered); updated: no
      // membership change, just the flash above.
      if (keysRef.current && !keysRef.current.includes(e.key)) {
        setKeys([...keysRef.current, e.key])
      }
    }
  })

  const openScope = (next: string) => {
    if (next === scope && !narrow) return
    if (!confirmDiscard()) return
    setScope(next)
    setKey(null)
  }
  const openKey = (next: string) => {
    if (next === key) return
    if (!confirmDiscard()) return
    setKey(next)
  }
  const backToScopes = () => {
    if (!confirmDiscard()) return
    setScope(null)
    setKey(null)
  }
  const backToKeys = () => {
    if (!confirmDiscard()) return
    setKey(null)
  }

  // Narrow: one pane at a time — scopes, keys, or the opened value.
  const showScopes = !narrow || scope === null
  const showKeys = !narrow || (scope !== null && key === null)
  const showDoc = !narrow || key !== null

  return (
    <div className={`state-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`} ref={rootRef}>
      {showScopes ? (
        <aside className="state-ui-col scopes" aria-label="scope list">
          <header className="state-ui-col-head">
            <span className="label">scopes</span>
            <span className="spacer" />
            {scopes !== null && !scopesError ? <span className="count">{scopes.length}</span> : null}
            <RefreshButton onClick={loadScopes} label="refresh scopes" />
          </header>
          <ColumnBody
            error={scopesError}
            noun="scope"
            onRetry={loadScopes}
            rows={scopes}
            selected={scope}
            flashed={scopeFlash}
            onOpen={openScope}
            empty={
              <div className="state-ui-col-empty">
                <p>No scopes yet.</p>
                <p className="dim">A scope appears here live with its first state::set.</p>
              </div>
            }
          />
        </aside>
      ) : null}

      {showKeys ? (
        <aside className="state-ui-col keys" aria-label="key list">
          {scope === null ? (
            <>
              <header className="state-ui-col-head">
                <span className="label">keys</span>
              </header>
              <div className="state-ui-col-hint">Select a scope to list its keys.</div>
            </>
          ) : (
            <>
              <header className="state-ui-col-head">
                {narrow ? <BackButton onClick={backToScopes} label="back to scopes" /> : null}
                <span className="scope-name" title={scope}>
                  {scope}
                </span>
                <span className="spacer" />
                {keys !== null && !keysError ? <span className="count">{keys.length}</span> : null}
                <RefreshButton onClick={loadKeys} label={`refresh keys of ${scope}`} />
              </header>
              <ColumnBody
                error={keysError}
                noun="key"
                onRetry={loadKeys}
                rows={keys}
                selected={key}
                flashed={keyFlash}
                onOpen={openKey}
                empty={
                  <div className="state-ui-col-empty">
                    <p>Scope is empty.</p>
                    <p className="dim">
                      New writes appear here live; the scope disappears once its last key is deleted.
                    </p>
                  </div>
                }
              />
            </>
          )}
        </aside>
      ) : null}

      {showDoc ? (
        <section className="state-ui-doc" aria-label="value workspace">
          {scope === null || key === null ? (
            <div className="state-ui-hero">
              <DatabaseIcon className="state-ui-hero-icon" />
              <h2 className="state-ui-hero-title">Select a key</h2>
              <p className="state-ui-hero-body">
                Pick a scope, then a key, to view and edit its JSON value. Edits save back with state::set; remote
                changes stream in live.
              </p>
              <p className="state-ui-hero-hint">
                <kbd>⌘S</kbd> saves
              </p>
            </div>
          ) : (
            <ValueEditor
              // Remount per entry so draft/save state never leaks across keys.
              key={`${scope}\u0000${key}`}
              host={host}
              scope={scope}
              itemKey={key}
              subscribe={subscribe}
              narrow={narrow}
              onBack={backToKeys}
              onDirtyChange={onDirtyChange}
            />
          )}
        </section>
      ) : null}
    </div>
  )
}
