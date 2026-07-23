import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, type Host } from '@iii-dev/console-ui'
import { type Subscribe, useStateEvents } from '../lib/events'
import { BackButton, LiveDot } from '../lib/widgets'

export function ItemView({
  host,
  scope,
  itemKey,
  subscribe,
  onBack,
}: {
  host: Host
  scope: string
  itemKey: string
  subscribe: Subscribe
  onBack: () => void
}) {
  const [stored, setStored] = useState<{ loaded: boolean; value: unknown }>({
    loaded: false,
    value: null,
  })
  const [loadError, setLoadError] = useState<string | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [savedAt, setSavedAt] = useState<number | null>(null)
  const [liveAt, setLiveAt] = useState<number | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [serverNotice, setServerNotice] = useState<
    { kind: 'changed'; value: unknown } | { kind: 'deleted' } | null
  >(null)
  const draftRef = useRef(draft)
  draftRef.current = draft

  useEffect(() => {
    host.iii
      .trigger<unknown>('state::get', { scope, key: itemKey })
      .then((value) => {
        setLoadError(null)
        setStored({ loaded: true, value })
      })
      .catch((err: unknown) =>
        setLoadError(err instanceof Error ? err.message : String(err)),
      )
  }, [host, scope, itemKey])

  const storedText = stored.loaded
    ? JSON.stringify(stored.value ?? null, null, 2)
    : ''

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
      const prevText = prev.loaded
        ? JSON.stringify(prev.value ?? null, null, 2)
        : null
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

  const text = draft ?? storedText
  const dirty = draft !== null && draft !== storedText

  let parseError: string | null = null
  if (dirty) {
    try {
      JSON.parse(text)
    } catch (err) {
      parseError = err instanceof Error ? err.message : String(err)
    }
  }

  const save = useCallback(async () => {
    setSaveError(null)
    let parsed: unknown
    try {
      parsed = JSON.parse(text)
    } catch {
      return
    }
    setSaving(true)
    try {
      await host.iii.trigger('state::set', {
        scope,
        key: itemKey,
        value: parsed,
      })
      setStored({ loaded: true, value: parsed })
      setDraft(null)
      setServerNotice(null)
      setSavedAt(Date.now())
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }, [host, scope, itemKey, text])

  const loadLatest = useCallback(() => {
    if (serverNotice?.kind !== 'changed') return
    setStored({ loaded: true, value: serverNotice.value ?? null })
    setDraft(null)
    setServerNotice(null)
    setLiveAt(Date.now())
  }, [serverNotice])

  return (
    <>
      <div className="state-ui-head">
        <BackButton onClick={onBack} label={`back to ${scope}`} />
        <span className="state-ui-crumb">
          {scope} <span className="sep">/</span> {itemKey}
        </span>
        <LiveDot />
        <span style={{ flex: 1 }} />
        <Button
          variant="primary"
          size="sm"
          onClick={() => void save()}
          disabled={!dirty || !!parseError || saving}
        >
          {saving ? 'saving…' : 'save'}
        </Button>
      </div>
      {loadError ? (
        <div className="state-ui-error">state::get failed — {loadError}</div>
      ) : !stored.loaded ? (
        <div className="state-ui-list">
          <div className="state-ui-note">loading value…</div>
        </div>
      ) : (
        <>
          <textarea
            className="state-ui-editor"
            value={text}
            spellCheck={false}
            onChange={(e) => {
              setDraft(e.target.value)
              setSavedAt(null)
            }}
            aria-label={`JSON value for ${scope}/${itemKey}`}
          />
          <div className="state-ui-editor-foot">
            {parseError ? (
              <span className="state-ui-invalid">invalid JSON — {parseError}</span>
            ) : saveError ? (
              <span className="state-ui-invalid">state::set failed — {saveError}</span>
            ) : serverNotice?.kind === 'deleted' ? (
              <span className="state-ui-server">
                this key was deleted on the server — saving will recreate it
              </span>
            ) : serverNotice?.kind === 'changed' ? (
              <>
                <span className="state-ui-server">
                  value changed on the server while you were editing
                </span>
                <button
                  type="button"
                  className="state-ui-linklike"
                  onClick={loadLatest}
                >
                  load latest (discards your edits)
                </button>
              </>
            ) : dirty ? (
              <span className="state-ui-dirty">unsaved changes</span>
            ) : savedAt ? (
              <span className="state-ui-saved">saved ✓</span>
            ) : liveAt ? (
              <span className="state-ui-saved">updated live ↻</span>
            ) : (
              <span className="state-ui-dirty">
                edit the JSON value and save to write it back with state::set —
                remote changes stream in live
              </span>
            )}
          </div>
        </>
      )}
    </>
  )
}
