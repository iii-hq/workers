/**
 * The generic list + editor browser both tabs are built on: a searchable
 * sidebar of entries, and an editor pane that round-trips one entry's RAW
 * markdown (frontmatter included) through the shared `CodeEditor` /
 * `MarkdownPreview` pair, saving over the worker's `*::update` functions.
 *
 * Live updates: the page registers a tab-scoped binding on the worker's
 * `directory::*::on-change` trigger type (see `useOnChange`) — a download
 * or update landing anywhere refreshes the list, and reloads the open
 * entry when the editor isn't dirty (an unsaved draft is never clobbered;
 * a note offers the reload instead).
 *
 * Layout adapts to the width the browser HAS (a ResizeObserver on its own
 * root, not a viewport media query — the console can host it in panes of
 * any size). Under NARROW_BELOW px it becomes a drill-in flow like the
 * state worker's page: the list fills the width, opening an entry swaps
 * the list for the editor with a ← back button, and split preview stacks
 * vertically.
 */

import {
  Button,
  CodeEditor,
  type Host,
  Input,
  MarkdownPreview,
} from '@iii-dev/console-ui'
import { useCallback, useEffect, useRef, useState } from 'react'
import { BackButton } from '../lib/widgets'

/** Container width (px) below which the browser collapses to the
 * drill-in list ⇄ editor flow. */
const NARROW_BELOW = 850

export interface BrowserRow {
  /** Stable key + the id/name passed to load/save. */
  key: string
  title: string
  description: string
  /** Fine-print line (size · modified). */
  fine: string
}

export interface BrowserAdapter {
  /** Singular noun for labels ("skill" / "prompt"). */
  noun: string
  list(host: Host): Promise<BrowserRow[]>
  /** Load one entry's full on-disk content (frontmatter included). */
  load(host: Host, key: string): Promise<string>
  /** Save; returns the entry's effective key after the write (a prompt
      rename via frontmatter `name:` moves the selection along). */
  save(host: Host, key: string, content: string): Promise<string>
  /** Worker trigger type to subscribe for external-change refreshes. */
  onChangeType: string
}

type EditorMode = 'edit' | 'split' | 'preview'

interface Loaded {
  key: string
  content: string
}

/** Observe the browser root's own width. Returns a callback ref to put
 * on the root plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash. */
function useContainerNarrow(
  threshold: number,
): [(node: HTMLDivElement | null) => void, boolean] {
  const [narrow, setNarrow] = useState(false)
  const observerRef = useRef<ResizeObserver | null>(null)
  const refCb = useCallback(
    (node: HTMLDivElement | null) => {
      observerRef.current?.disconnect()
      observerRef.current = null
      if (!node) return
      setNarrow(node.getBoundingClientRect().width < threshold)
      const observer = new ResizeObserver((entries) => {
        const width = entries[0]?.contentRect.width
        if (typeof width === 'number') setNarrow(width < threshold)
      })
      observer.observe(node)
      observerRef.current = observer
    },
    [threshold],
  )
  return [refCb, narrow]
}

/** Tab-scoped subscription to one of the worker's on-change trigger
 * types. The handler id carries the `iii::` prefix so per-event
 * invocations stay span-suppressed and out of the trace feed; the
 * binding is GC'd with the tab and unregistered on unmount. */
function useOnChange(host: Host, triggerType: string, onEvent: () => void) {
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  useEffect(() => {
    const slug = triggerType.replace(/[^a-z0-9]+/g, '-')
    const fnId = `iii::iii-directory-ui::${slug}::${host.iii.browserId}`
    const offHandler = host.iii.on(fnId, () => {
      onEventRef.current()
    })
    const offTrigger = host.iii.registerTrigger({
      type: triggerType,
      function_id: fnId,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, triggerType])
}

export function CollectionBrowser({
  host,
  adapter,
}: {
  host: Host
  adapter: BrowserAdapter
}) {
  const [rows, setRows] = useState<BrowserRow[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [search, setSearch] = useState('')

  const [selected, setSelected] = useState<string | null>(null)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [draft, setDraft] = useState('')
  const [loadError, setLoadError] = useState<string | null>(null)

  const [mode, setMode] = useState<EditorMode>('split')
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [savedFlash, setSavedFlash] = useState(false)
  const [staleOnDisk, setStaleOnDisk] = useState(false)

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)

  const dirty = loaded !== null && draft !== loaded.content

  const refreshList = useCallback(() => {
    adapter
      .list(host)
      .then((next) => {
        setRows(next)
        setListError(null)
      })
      .catch((e) => setListError(String(e)))
  }, [host, adapter])

  useEffect(() => {
    refreshList()
  }, [refreshList])

  const dirtyRef = useRef(dirty)
  dirtyRef.current = dirty
  const selectedRef = useRef(selected)
  selectedRef.current = selected

  const open = useCallback(
    (key: string) => {
      setSelected(key)
      setLoaded(null)
      setDraft('')
      setLoadError(null)
      setSaveError(null)
      setStaleOnDisk(false)
      adapter
        .load(host, key)
        .then((content) => {
          // Stale async result: the user opened another entry (or drilled
          // out) while this load was in flight — applying it would show A's
          // content under B's title and risk saving it there.
          if (selectedRef.current !== key) return
          setLoaded({ key, content })
          setDraft(content)
        })
        .catch((e) => {
          if (selectedRef.current === key) setLoadError(String(e))
        })
    },
    [host, adapter],
  )

  // External change (download / update from anywhere): refresh the list;
  // reload the open entry only when the editor has no unsaved edits.
  useOnChange(host, adapter.onChangeType, () => {
    refreshList()
    const key = selectedRef.current
    if (!key) return
    if (dirtyRef.current) {
      setStaleOnDisk(true)
      return
    }
    adapter
      .load(host, key)
      .then((content) => {
        setLoaded((prev) =>
          prev && prev.key === key ? { key, content } : prev,
        )
        setDraft((prev) => {
          const current = selectedRef.current
          return current === key ? content : prev
        })
      })
      .catch(() => {
        /* entry may have been removed; the refreshed list reflects it */
      })
  })

  const save = useCallback(() => {
    if (!loaded || saving || draft === loaded.content) return
    const key = loaded.key
    setSaving(true)
    setSaveError(null)
    adapter
      .save(host, key, draft)
      .then((effectiveKey) => {
        // The write happened either way; only the editor state is stale if
        // the user navigated away mid-save — don't yank them back to the
        // saved entry or overwrite what they're looking at now.
        refreshList()
        if (selectedRef.current !== key) return
        setLoaded({ key: effectiveKey, content: draft })
        setSelected(effectiveKey)
        setStaleOnDisk(false)
        setSavedFlash(true)
        window.setTimeout(() => setSavedFlash(false), 1600)
      })
      .catch((e) => {
        if (selectedRef.current === key) setSaveError(String(e))
      })
      .finally(() => setSaving(false))
  }, [host, adapter, loaded, draft, saving, refreshList])

  const onEditorKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
      e.preventDefault()
      save()
    }
  }

  // Narrow-mode drill-out: back to the list, guarding an unsaved draft.
  const goBack = () => {
    if (dirty && !window.confirm('Discard unsaved changes?')) return
    setSelected(null)
    setLoaded(null)
    setDraft('')
    setLoadError(null)
    setSaveError(null)
    setStaleOnDisk(false)
  }

  const needle = search.trim().toLowerCase()
  const visible = (rows ?? []).filter(
    (r) =>
      !needle ||
      r.key.toLowerCase().includes(needle) ||
      r.title.toLowerCase().includes(needle) ||
      r.description.toLowerCase().includes(needle),
  )

  // Narrow: one pane at a time — the list, or the opened entry.
  const showSide = !narrow || selected === null
  const showEditor = !narrow || selected !== null

  return (
    <div
      className={`dir-ui-browser${narrow ? ' narrow' : ''}`}
      ref={rootRef}
    >
      {showSide ? (
      <div className="dir-ui-side">
        <div className="dir-ui-side-search">
          <Input
            value={search}
            onChange={setSearch}
            placeholder={`filter ${adapter.noun}s…`}
            aria-label={`filter ${adapter.noun}s`}
          />
        </div>
        {listError ? (
          <div className="dir-ui-error">{listError}</div>
        ) : rows === null ? (
          <div className="dir-ui-pulse">· loading {adapter.noun}s…</div>
        ) : visible.length === 0 ? (
          <div className="dir-ui-empty">
            · no {adapter.noun}s{needle ? ' match' : ''}
          </div>
        ) : (
          <ul className="dir-ui-side-list">
            {visible.map((r) => (
              <li key={r.key}>
                <button
                  type="button"
                  className={`dir-ui-side-row${
                    r.key === selected ? ' active' : ''
                  }`}
                  onClick={() => open(r.key)}
                >
                  <span className="dir-ui-id">{r.key}</span>
                  {r.title && r.title !== r.key ? (
                    <span className="dir-ui-title">{r.title}</span>
                  ) : null}
                  {r.description ? (
                    <span className="dir-ui-desc clamp">{r.description}</span>
                  ) : null}
                  <span className="dir-ui-fine">{r.fine}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      ) : null}

      {showEditor ? (
      <div className="dir-ui-editor">
        {narrow && selected !== null && loaded === null ? (
          <div className="dir-ui-editor-head">
            <BackButton
              onClick={goBack}
              label={`back to ${adapter.noun} list`}
            />
            <span className="dir-ui-id lg grow">{selected}</span>
          </div>
        ) : null}
        {selected === null ? (
          <div className="dir-ui-empty pad">
            · select a {adapter.noun} to view and edit its markdown
          </div>
        ) : loadError ? (
          <div className="dir-ui-error">{loadError}</div>
        ) : loaded === null ? (
          <div className="dir-ui-pulse">· loading {selected}…</div>
        ) : (
          <>
            <div className="dir-ui-editor-head">
              {narrow ? (
                <BackButton
                  onClick={goBack}
                  label={`back to ${adapter.noun} list`}
                />
              ) : null}
              <span className="dir-ui-id lg grow">{loaded.key}</span>
              <span className="dir-ui-modes" role="group" aria-label="editor mode">
                {(['edit', 'split', 'preview'] as const).map((m) => (
                  <Button
                    key={m}
                    variant="ghost"
                    size="sm"
                    aria-pressed={mode === m}
                    className={mode === m ? 'dir-ui-mode-active' : undefined}
                    onClick={() => setMode(m)}
                  >
                    {m}
                  </Button>
                ))}
              </span>
              {dirty ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={saving}
                  onClick={() => {
                    setDraft(loaded.content)
                    setSaveError(null)
                  }}
                >
                  revert
                </Button>
              ) : null}
              <Button
                variant="primary"
                size="sm"
                disabled={!dirty || saving}
                onClick={save}
              >
                {saving ? 'saving…' : 'save'}
              </Button>
              <span className="dir-ui-save-note" aria-live="polite">
                {saving
                  ? ''
                  : savedFlash
                    ? 'saved'
                    : dirty
                      ? 'unsaved changes · ⌘S saves'
                      : ''}
              </span>
            </div>
            {staleOnDisk ? (
              <div className="dir-ui-stale-note">
                changed on disk while you were editing —{' '}
                <button
                  type="button"
                  className="dir-ui-linkish"
                  onClick={() => open(loaded.key)}
                >
                  reload (discards your draft)
                </button>
              </div>
            ) : null}
            <div
              className={`dir-ui-editor-body mode-${mode}`}
              onKeyDown={onEditorKeyDown}
            >
              {mode !== 'preview' ? (
                <div className="dir-ui-pane">
                  <CodeEditor
                    value={draft}
                    onChange={setDraft}
                    language="markdown"
                    className="dir-ui-code"
                    aria-label={`${adapter.noun} markdown source`}
                    placeholder="# markdown…"
                  />
                </div>
              ) : null}
              {mode !== 'edit' ? (
                <div className="dir-ui-pane">
                  <MarkdownPreview
                    markdown={stripFrontmatter(draft)}
                    className="dir-ui-preview"
                  />
                </div>
              ) : null}
            </div>
            {saveError ? <div className="dir-ui-error">{saveError}</div> : null}
          </>
        )}
      </div>
      ) : null}
    </div>
  )
}

/** Preview the body the way readers serve it: without the leading YAML
 * frontmatter block (same fence rules as the worker's split_frontmatter). */
function stripFrontmatter(content: string): string {
  if (!content.startsWith('---\n') && !content.startsWith('---\r\n')) {
    return content
  }
  const rest = content.slice(content.indexOf('\n') + 1)
  for (const fence of ['\n---\n', '\n---\r\n']) {
    const idx = rest.indexOf(fence)
    if (idx !== -1) return rest.slice(idx + fence.length)
  }
  return content
}
