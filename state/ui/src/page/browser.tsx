/**
 * The scopes × keys browser the state page is built on.
 *
 * Layout adapts to the width the browser HAS (`useContainerNarrow` on its
 * own root, not a viewport media query — the console can host it in panes
 * of any size). Wide: three columns — scopes, the selected scope's keys,
 * and the value workspace side by side. Under NARROW_BELOW px it becomes a
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

import {
  Button,
  EmptyState,
  type Host,
  IconButton,
  KeyCombo,
  List,
  ListItem,
  type PageCommandsApi,
  PageSidebar,
  type PanelContextEvent,
  Skeleton,
  StatusPanel,
  uiClasses,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Database, RefreshCw } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type Subscribe, useStateEvents } from '../lib/events'
import { parseStatePanelContext } from './panel-context'
import { ValueEditor } from './ValueEditor'

/** Container width (px) below which the browser collapses to the
 * drill-in scopes ⇄ keys ⇄ value flow. */
const NARROW_BELOW = 800

/** Transient per-row highlight for live-arrived changes. */
function useFlash(): [ReadonlySet<string>, (k: string) => void] {
  const [flashed, setFlashed] = useState<ReadonlySet<string>>(new Set())
  const mark = useCallback((k: string) => {
    setFlashed((prev) => new Set(prev).add(k))
    window.setTimeout(() => {
      setFlashed((prev) => {
        const next = new Set(prev)
        next.delete(k)
        return next
      })
    }, 1400)
  }, [])
  return [flashed, mark]
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
        <StatusPanel
          variant="alert"
          headline={`The ${noun} list could not be loaded.`}
          detail={error}
          action={
            <Button variant="ghost" size="sm" onClick={onRetry}>
              retry
            </Button>
          }
        />
      ) : rows === null ? (
        <div className="state-ui-col-skel" aria-hidden>
          {[60, 80, 40, 70].map((w) => (
            <Skeleton key={w} className="state-ui-skel" style={{ width: `${w}%` }} />
          ))}
        </div>
      ) : rows.length === 0 ? (
        empty
      ) : (
        <List>
          {rows.map((row) => (
            <ListItem
              key={row}
              selected={row === selected}
              aria-current={row === selected ? 'true' : undefined}
              className={flashed.has(row) ? 'state-ui-flash' : undefined}
              onClick={() => onOpen(row)}
              label={<span className="state-ui-name">{row}</span>}
            />
          ))}
        </List>
      )}
    </div>
  )
}

export function StateBrowser({
  host,
  subscribe,
  panelSide = 'left',
  panelContext,
  commands,
}: {
  host: Host
  subscribe: Subscribe
  panelSide?: 'left' | 'right'
  panelContext?: PanelContextEvent
  commands?: PageCommandsApi
}) {
  const [scopes, setScopes] = useState<string[] | null>(null)
  const [scopesError, setScopesError] = useState<string | null>(null)
  const [scopeFlash, flashScope] = useFlash()

  const [scope, setScope] = useState<string | null>(null)
  const [key, setKey] = useState<string | null>(null)

  const [keys, setKeys] = useState<string[] | null>(null)
  const [keysError, setKeysError] = useState<string | null>(null)
  const [keyFlash, flashKey] = useFlash()

  const { ref: rootRef, narrow } = useContainerNarrow({ below: NARROW_BELOW })

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
  const { confirm, dialog } = useConfirm()
  const confirmDiscard = async () =>
    !dirtyRef.current || confirm({ title: 'Discard unsaved changes?', confirmLabel: 'Discard', tone: 'danger' })

  const loadScopes = useCallback(() => {
    host.iii
      .trigger<{ groups: string[] }>('state::list_groups', {})
      .then((r) => {
        setScopesError(null)
        setScopes(r.groups)
      })
      .catch((err: unknown) => setScopesError(errorMessage(err)))
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
      if (refetchTimer.current !== null)
        window.clearTimeout(refetchTimer.current)
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
        setKeysError(errorMessage(err))
      })
  }, [host])
  useEffect(() => {
    setKeys(null)
    setKeysError(null)
    if (scope !== null) loadKeys()
  }, [scope, loadKeys])

  // A context from the palette source (or another worker's "inspect this
  // key" affordance) selects a scope, then its key, as each list loads —
  // the page can mount before the first fetch resolves.
  const appliedContextRef = useRef(0)
  useEffect(() => {
    if (!panelContext || panelContext.id === appliedContextRef.current) return
    const context = parseStatePanelContext(panelContext.context)
    if (!context) {
      appliedContextRef.current = panelContext.id
      return
    }
    if (scopes === null) return
    if (!scopes.includes(context.scope)) {
      appliedContextRef.current = panelContext.id
      return
    }
    if (scope !== context.scope) {
      setScope(context.scope)
      setKey(null)
      return
    }
    if (keys === null) return
    if (!keys.includes(context.key)) {
      appliedContextRef.current = panelContext.id
      return
    }
    appliedContextRef.current = panelContext.id
    setKey(context.key)
  }, [panelContext, scopes, scope, keys])

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

  const openScope = async (next: string) => {
    if (next === scope && !narrow) return
    if (!(await confirmDiscard())) return
    setScope(next)
    setKey(null)
  }
  const openKey = async (next: string) => {
    if (next === key) return
    if (!(await confirmDiscard())) return
    setKey(next)
  }
  const backToScopes = async () => {
    if (!(await confirmDiscard())) return
    setScope(null)
    setKey(null)
  }
  const backToKeys = async () => {
    if (!(await confirmDiscard())) return
    setKey(null)
  }

  // Narrow: one pane at a time — scopes, keys, or the opened value.
  const showScopes = !narrow || scope === null
  const showKeys = !narrow || (scope !== null && key === null)
  const showDoc = !narrow || key !== null

  return (
    <div
      className={`state-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
      ref={rootRef}
    >
      {dialog}
      {showScopes ? (
        <PageSidebar
          label="scopes"
          side={panelSide}
          collapsible
          storageKey="state:scopes"
          defaultWidth={208}
          narrow={narrow}
          className="state-ui-col scopes"
          data-autofocus=""
          tabIndex={-1}
          header={
            <div className="state-ui-col-head state-ui-primary-head">
              <span className={uiClasses.eyebrow}>scopes</span>
              <span className="spacer" />
              {scopes !== null && !scopesError ? (
                <span className="count">{scopes.length}</span>
              ) : null}
              <IconButton label="refresh scopes" onClick={loadScopes}>
                <RefreshCw size={16} aria-hidden />
              </IconButton>
            </div>
          }
        >
          <ColumnBody
            error={scopesError}
            noun="scope"
            onRetry={loadScopes}
            rows={scopes}
            selected={scope}
            flashed={scopeFlash}
            onOpen={openScope}
            empty={
              <EmptyState
                compact
                title="No scopes yet."
                description="A scope appears here live with its first state::set."
              />
            }
          />
        </PageSidebar>
      ) : null}

      {showKeys ? (
        <aside className="state-ui-col keys" aria-label="key list">
          {scope === null ? (
            <>
              <header className="state-ui-col-head">
                <span className={uiClasses.eyebrow}>keys</span>
              </header>
              <div className="state-ui-col-hint">
                Select a scope to list its keys.
              </div>
            </>
          ) : (
            <>
              <header className="state-ui-col-head">
                {narrow ? (
                  <IconButton label="back to scopes" onClick={backToScopes}>
                    <ArrowLeft size={16} aria-hidden />
                  </IconButton>
                ) : null}
                <span className="scope-name" title={scope}>
                  {scope}
                </span>
                <span className="spacer" />
                {keys !== null && !keysError ? (
                  <span className="count">{keys.length}</span>
                ) : null}
                <IconButton label={`refresh keys of ${scope}`} onClick={loadKeys}>
                  <RefreshCw size={16} aria-hidden />
                </IconButton>
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
                  <EmptyState
                    compact
                    title="Scope is empty."
                    description="New writes appear here live; the scope disappears once its last key is deleted."
                  />
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
              <EmptyState
                icon={Database}
                title="Select a key"
                description="Pick a scope, then a key, to view and edit its JSON value. Edits save back with state::set; remote changes stream in live."
              />
              <p className="state-ui-hero-hint">
                <KeyCombo binding="Mod+S" /> saves
              </p>
            </div>
          ) : (
            <ValueEditor
              // Remount per entry so draft/save state never leaks across keys.
              key={`${scope} ${key}`}
              host={host}
              scope={scope}
              itemKey={key}
              subscribe={subscribe}
              narrow={narrow}
              onBack={backToKeys}
              onDirtyChange={onDirtyChange}
              commands={commands}
            />
          )}
        </section>
      ) : null}
    </div>
  )
}
