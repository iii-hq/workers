/**
 * The value workspace: one scope/key's JSON value in the console's shared
 * Monaco editor (CodeEditor from @iii-dev/console-ui — never bundle an
 * editor into a worker asset), under a stable document header (identity
 * crumb, dirty dot, revert/save) and over a status bar.
 *
 * Live semantics (the page-wide `state` binding, filtered to this entry):
 * a clean editor applies server changes in place; an editor with unsaved
 * edits is never clobbered — a banner offers the new value instead. A
 * delete keeps the draft and warns that saving recreates the key.
 *
 * The browser remounts this component per scope/key (React key), so all
 * state here is entry-local. Unsaved edits are reported up through
 * `onDirtyChange` so column navigation can ask before discarding them.
 */

import { Button, CodeEditor, type Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { type Subscribe, useStateEvents } from '../lib/events'
import { BackButton } from '../lib/widgets'

export function ValueEditor({
  host,
  scope,
  itemKey,
  subscribe,
  narrow,
  onBack,
  onDirtyChange,
}: {
  host: Host
  scope: string
  itemKey: string
  subscribe: Subscribe
  narrow: boolean
  onBack: () => void
  onDirtyChange: (dirty: boolean) => void
}) {
  const [stored, setStored] = useState<{ loaded: boolean; value: unknown }>({
    loaded: false,
    value: null,
  })
  const [loadError, setLoadError] = useState<string | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [savedFlash, setSavedFlash] = useState(false)
  const [liveAt, setLiveAt] = useState<number | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [serverNotice, setServerNotice] = useState<
    { kind: 'changed'; value: unknown } | { kind: 'deleted' } | null
  >(null)
  const draftRef = useRef(draft)
  draftRef.current = draft
  const flashTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(flashTimer.current), [])

  const load = useCallback(() => {
    setLoadError(null)
    host.iii
      .trigger<unknown>('state::get', { scope, key: itemKey })
      .then((value) => {
        setLoadError(null)
        setStored({ loaded: true, value })
      })
      .catch((err: unknown) => setLoadError(err instanceof Error ? err.message : String(err)))
  }, [host, scope, itemKey])
  useEffect(load, [load])

  const storedText = stored.loaded ? JSON.stringify(stored.value ?? null, null, 2) : ''
  const text = draft ?? storedText
  const dirty = draft !== null && draft !== storedText

  const onDirtyChangeRef = useRef(onDirtyChange)
  onDirtyChangeRef.current = onDirtyChange
  useEffect(() => {
    onDirtyChangeRef.current(dirty)
  }, [dirty])
  // Unmount (back, scope switch, page close) always releases the guard.
  useEffect(() => () => onDirtyChangeRef.current(false), [])

  useStateEvents(subscribe, (e) => {
    if (e.scope !== scope || e.key !== itemKey) return
    if (e.event_type === 'state:deleted') {
      setServerNotice({ kind: 'deleted' })
      return
    }
    const nextText = JSON.stringify(e.new_value ?? null, null, 2)
    if (draftRef.current !== null && draftRef.current !== nextText) {
      // Never clobber an editor with unsaved changes — offer the new
      // value instead.
      setServerNotice({ kind: 'changed', value: e.new_value })
      return
    }
    // Clean editor (or a draft that already equals the incoming value —
    // typically the echo of our own save): apply in place.
    setServerNotice(null)
    setStored((prev) => {
      const prevText = prev.loaded ? JSON.stringify(prev.value ?? null, null, 2) : null
      if (prevText !== nextText) setLiveAt(Date.now())
      return { loaded: true, value: e.new_value ?? null }
    })
    if (draftRef.current === nextText) setDraft(null)
  })

  // The transient "updated live" note fades back to the default hint.
  useEffect(() => {
    if (liveAt === null) return
    const timer = window.setTimeout(() => setLiveAt(null), 4000)
    return () => window.clearTimeout(timer)
  }, [liveAt])

  let parseError: string | null = null
  if (dirty) {
    try {
      JSON.parse(text)
    } catch (err) {
      parseError = err instanceof Error ? err.message : String(err)
    }
  }

  const save = useCallback(async () => {
    if (saving) return
    setSaveError(null)
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch {
      return
    }
    setSaving(true)
    try {
      await host.iii.trigger('state::set', { scope, key: itemKey, value: parsed })
      setStored({ loaded: true, value: parsed })
      setDraft(null)
      setServerNotice(null)
      setSavedFlash(true)
      window.clearTimeout(flashTimer.current)
      flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2400)
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }, [host, scope, itemKey, text, saving])

  const loadLatest = useCallback(() => {
    if (serverNotice?.kind !== 'changed') return
    setStored({ loaded: true, value: serverNotice.value ?? null })
    setDraft(null)
    setServerNotice(null)
    setLiveAt(Date.now())
  }, [serverNotice])

  const onWorkspaceKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
      e.preventDefault()
      if (dirty && !parseError) void save()
    }
  }

  const lines = stored.loaded ? text.split('\n').length : 0
  const bytes = stored.loaded ? new Blob([text]).size : 0
  const saveStatus = saving ? 'saving…' : savedFlash ? '✓ saved' : dirty ? 'unsaved' : ''

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay (⌘S saves) around real controls
    <div className="state-ui-doc-inner" onKeyDown={onWorkspaceKeyDown}>
      <header className="state-ui-doc-head">
        {narrow ? <BackButton onClick={onBack} label={`back to ${scope}`} /> : null}
        <div className="state-ui-doc-identity">
          <span className="state-ui-doc-name" title={`${scope} / ${itemKey}`}>
            <span className="txt">{itemKey}</span>
            {dirty ? <span className="state-ui-dirty-dot" title="unsaved changes" aria-hidden /> : null}
          </span>
          {!narrow ? (
            <span className="state-ui-doc-crumb">
              {scope} / {itemKey}
            </span>
          ) : null}
        </div>
        <div className="state-ui-save-area">
          <span className="state-ui-save-note" aria-live="polite">
            {saveStatus}
          </span>
          {dirty && !saving ? (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setDraft(null)
                setSaveError(null)
              }}
            >
              revert
            </Button>
          ) : null}
          <Button variant="primary" size="sm" disabled={!dirty || !!parseError || saving} onClick={() => void save()}>
            {saving ? 'saving…' : 'save'}
          </Button>
        </div>
      </header>

      {serverNotice?.kind === 'deleted' ? (
        <div className="state-ui-banner warn" role="status">
          <span>This key was deleted on the server — saving recreates it.</span>
        </div>
      ) : null}

      {serverNotice?.kind === 'changed' ? (
        <div className="state-ui-banner warn" role="status">
          <span>This value changed on the server while you were editing.</span>
          <button type="button" className="state-ui-linkish" onClick={loadLatest}>
            load latest (discards your draft)
          </button>
        </div>
      ) : null}

      {saveError ? (
        <div className="state-ui-banner alert" role="alert">
          <span>
            state::set failed.
            <span className="detail">{saveError}</span>
          </span>
          <button type="button" className="state-ui-linkish" onClick={() => void save()}>
            retry
          </button>
          <button type="button" className="state-ui-linkish quiet" onClick={() => setSaveError(null)}>
            dismiss
          </button>
        </div>
      ) : null}

      {loadError ? (
        <div className="state-ui-error-panel grow">
          <p>
            {scope} / {itemKey} could not be loaded.
            <span className="detail">{loadError}</span>
          </p>
          <Button variant="ghost" size="sm" onClick={load}>
            retry
          </Button>
        </div>
      ) : !stored.loaded ? (
        <div className="state-ui-doc-loading" aria-hidden>
          <span className="bar w40" />
          <span className="bar w90" />
          <span className="bar w75" />
          <span className="bar w30" />
        </div>
      ) : (
        <>
          <div className="state-ui-doc-body">
            <CodeEditor
              value={text}
              onChange={(next) => setDraft(next)}
              language="json"
              className="state-ui-code"
              aria-label={`JSON value for ${scope}/${itemKey}`}
              placeholder="null"
            />
          </div>
          <footer className="state-ui-statusbar">
            <span className="fact">json</span>
            <span className="fact">
              {lines} line{lines === 1 ? '' : 's'}
            </span>
            <span className="fact">{formatKb(bytes)}</span>
            <span className="spacer" />
            {parseError ? (
              <span className="status alert" title={parseError}>
                invalid JSON — {parseError}
              </span>
            ) : dirty ? (
              <span className="status">⌘S saves</span>
            ) : liveAt ? (
              <span className="status ok">updated live ↻</span>
            ) : (
              <span className="status">all changes saved</span>
            )}
          </footer>
        </>
      )}
    </div>
  )
}

function formatKb(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  return `${(bytes / 1024).toFixed(1)} KB`
}
