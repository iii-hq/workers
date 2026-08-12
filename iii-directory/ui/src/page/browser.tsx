/**
 * The generic list + editor browser both collections are built on: a
 * navigation sidebar (collection switcher, search, count, entry list) and
 * a document workspace with a stable header (identity crumb, segmented
 * Edit/Split/Preview control, save state) over the shared `CodeEditor` /
 * `MarkdownPreview` pair, saving through the worker's `*::update`
 * functions.
 *
 * Live updates: the page registers a tab-scoped binding on the worker's
 * `directory::*::on-change` trigger type (see `useOnChange`) — a download
 * or update landing anywhere refreshes the list, and reloads the open
 * entry when the editor isn't dirty (an unsaved draft is never clobbered;
 * a banner offers the reload instead).
 *
 * Layout adapts to the width the browser HAS (a ResizeObserver on its own
 * root, not a viewport media query — the console can host it in panes of
 * any size). Under NARROW_BELOW px it becomes a drill-in flow: the list
 * fills the width, opening an entry swaps the list for the document with
 * a ← back button, and split mode is unavailable (edit/preview only).
 *
 * The editor pane stays MOUNTED across mode switches (hidden in preview
 * mode) so Monaco keeps cursor and scroll position; the split divider is
 * draggable and its ratio, like the view mode, persists per tab+collection
 * in localStorage.
 */

import { Button, CodeEditor, type Host, Input, MarkdownPreview } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { BackButton, MarkdownFileIcon, PlusIcon, SearchIcon, XIcon } from '../lib/widgets'
import {
  frontmatterBody,
  readFrontmatterField,
  restoreFrontmatterFields,
  setFrontmatterField,
  withoutFrontmatterFields,
} from './frontmatter'

/** Container width (px) below which the browser collapses to the
 * drill-in list ⇄ document flow. */
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
  /** Breadcrumb root segment ("skills" / "prompts"). */
  crumbRoot: string
  /** Frontmatter keys that may carry the displayed name, in precedence
      order. Skills support both legacy `title` and standard `name`. */
  nameKeys?: readonly string[]
  /** Field inserted when the document has no declared name yet. */
  defaultNameKey?: string
  /** Prompt names use the worker's lowercase identifier grammar. */
  slugName?: boolean
  /** Prompt scanners reject an empty description; skills keep it optional. */
  descriptionRequired?: boolean
  /** Workspace empty-state copy. */
  emptyTitle: string
  emptyBody: string
  list(host: Host): Promise<BrowserRow[]>
  /** Load one entry's full on-disk content (frontmatter included). */
  load(host: Host, key: string): Promise<string>
  /** Save; returns the entry's effective key after the write (a prompt
      rename via frontmatter `name:` moves the selection along). */
  save(host: Host, key: string, content: string): Promise<string>
  /** Create a NEW entry; returns its key. Omit to hide the "new" button —
      skills arrive by download, so only prompt-ish collections set this. */
  create?(host: Host, name: string, content: string): Promise<string>
  /** Permanently remove an existing entry. Omit when deletion is unsupported. */
  remove?(host: Host, key: string): Promise<void>
  /** Worker trigger type to subscribe for external-change refreshes. */
  onChangeType: string
}

/** Starter frontmatter for a new entry — the scanner rejects a file without
 *  a `description`, so scaffold both required keys rather than a blank page. */
const NEW_TEMPLATE = '---\nname: \ndescription: ""\n---\n\n'
const SLUG_NAME = /^[a-z0-9_-]+$/

type EditorMode = 'edit' | 'split' | 'preview'

interface Loaded {
  key: string
  content: string
}

function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key)
  } catch {
    return null
  }
}

function writeStored(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    /* private mode / quota — persistence is best-effort */
  }
}

const clampRatio = (n: number) => Math.min(0.75, Math.max(0.25, n))

/** Observe the browser root's own width. Returns a callback ref to put
 * on the root plus whether it is currently narrower than `threshold` —
 * container-driven, so the same page adapts inside any pane the console
 * gives it. Measures synchronously on mount to avoid a wide-mode flash;
 * zero widths (display:none — the inactive collection) are ignored so a
 * hidden browser keeps its last real layout. */
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
  nav,
  panelSide = 'left',
  storageKey,
}: {
  host: Host
  adapter: BrowserAdapter
  /** Sidebar top slot — the skills/prompts switcher lives here. */
  nav?: ReactNode
  panelSide?: 'left' | 'right'
  /** localStorage namespace for per-tab+collection UI state. */
  storageKey: string
}) {
  const [rows, setRows] = useState<BrowserRow[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const searchRef = useRef<HTMLInputElement | null>(null)
  const fieldId = useId()

  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [draft, setDraft] = useState('')
  const [loadError, setLoadError] = useState<string | null>(null)

  const [mode, setModeState] = useState<EditorMode>(() => {
    const v = readStored(`${storageKey}:mode`)
    return v === 'edit' || v === 'split' || v === 'preview' ? v : 'split'
  })
  const setMode = useCallback(
    (m: EditorMode) => {
      setModeState(m)
      writeStored(`${storageKey}:mode`, m)
    },
    [storageKey],
  )

  const [split, setSplit] = useState(() => {
    const v = Number.parseFloat(readStored(`${storageKey}:split`) ?? '')
    return Number.isFinite(v) ? clampRatio(v) : 0.5
  })
  const [dragging, setDragging] = useState(false)

  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [savedFlash, setSavedFlash] = useState(false)
  const [staleOnDisk, setStaleOnDisk] = useState(false)
  const flashTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(flashTimer.current), [])

  const [rootRef, narrow] = useContainerNarrow(NARROW_BELOW)
  const bodyRef = useRef<HTMLDivElement | null>(null)

  const dirty = loaded !== null && draft !== loaded.content
  // Split needs side-by-side room; narrow containers fall back to edit.
  const effMode: EditorMode = narrow && mode === 'split' ? 'edit' : mode

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
  const creatingRef = useRef(creating)
  creatingRef.current = creating

  const open = useCallback(
    (key: string, opts?: { reload?: boolean }) => {
      if (!opts?.reload) {
        // Re-clicking the open entry must not clobber the draft; switching
        // away from an unsaved draft asks first.
        if (key === selectedRef.current) return
        if (dirtyRef.current && !window.confirm(`Discard unsaved changes to ${selectedRef.current}?`)) {
          return
        }
      }
      setSelected(key)
      setCreating(false)
      setLoaded(null)
      setDraft('')
      setLoadError(null)
      setSaveError(null)
      setDeleteError(null)
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

  /** Blank editor for a new entry. `loaded.content` is empty rather than the
      template, so the scaffold already counts as unsaved work and ⌘S/save is
      live immediately. */
  const startCreate = useCallback(() => {
    if (
      dirtyRef.current &&
      !window.confirm(`Discard unsaved changes to ${selectedRef.current}?`)
    ) {
      return
    }
    setCreating(true)
    setSelected(null)
    setLoaded({ key: '', content: '' })
    setDraft(NEW_TEMPLATE)
    setLoadError(null)
    setSaveError(null)
    setDeleteError(null)
    setStaleOnDisk(false)
  }, [])

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
        setLoaded((prev) => (prev && prev.key === key ? { key, content } : prev))
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
    if (!loaded || saving || deleting || draft === loaded.content) return
    const name = readFrontmatterField(
      draft,
      adapter.nameKeys ?? ['name'],
      adapter.defaultNameKey,
    ).value.trim()
    const effectiveName = name || (!creating ? loaded.key : '')
    if (adapter.slugName && !SLUG_NAME.test(effectiveName)) {
      setSaveError(
        'enter a name using lowercase letters, numbers, hyphens or underscores',
      )
      return
    }
    const description = readFrontmatterField(draft, ['description']).value
    if (adapter.descriptionRequired && !description.trim()) {
      setSaveError('enter a description before saving')
      return
    }
    if (creating) {
      if (!adapter.create) return
      setSaving(true)
      setSaveError(null)
      setDeleteError(null)
      adapter
        .create(host, effectiveName, draft)
        .then((effectiveKey) => {
          refreshList()
          if (!creatingRef.current) return
          setCreating(false)
          setLoaded({ key: effectiveKey, content: draft })
          setSelected(effectiveKey)
          setStaleOnDisk(false)
          setSavedFlash(true)
          window.clearTimeout(flashTimer.current)
          flashTimer.current = window.setTimeout(
            () => setSavedFlash(false),
            2400,
          )
        })
        .catch((e) => {
          if (creatingRef.current) setSaveError(String(e))
        })
        .finally(() => setSaving(false))
      return
    }
    const key = loaded.key
    setSaving(true)
    setSaveError(null)
    setDeleteError(null)
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
        window.clearTimeout(flashTimer.current)
        flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2400)
      })
      .catch((e) => {
        if (selectedRef.current === key) setSaveError(String(e))
      })
      .finally(() => setSaving(false))
  }, [host, adapter, loaded, draft, saving, deleting, creating, refreshList])

  const remove = useCallback(() => {
    if (!adapter.remove || !loaded || creating || saving || deleting) return
    const key = loaded.key
    const dirtyNote = dirty ? '\n\nYour unsaved changes will also be lost.' : ''
    if (
      !window.confirm(
        `Delete ${adapter.noun} “${key}”? This removes its file and cannot be undone.${dirtyNote}`,
      )
    ) {
      return
    }

    setDeleting(true)
    setDeleteError(null)
    setSaveError(null)
    adapter
      .remove(host, key)
      .then(() => {
        setRows((current) => current?.filter((row) => row.key !== key) ?? null)
        refreshList()
        if (selectedRef.current !== key) return
        setSelected(null)
        setLoaded(null)
        setDraft('')
        setLoadError(null)
        setStaleOnDisk(false)
      })
      .catch((error) => {
        if (selectedRef.current === key) setDeleteError(String(error))
      })
      .finally(() => setDeleting(false))
  }, [host, adapter, loaded, creating, saving, deleting, dirty, refreshList])

  const onWorkspaceKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
      e.preventDefault()
      save()
    }
  }

  // `/` anywhere outside a text surface jumps to the list filter.
  const onRootKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return
    const t = e.target as HTMLElement
    if (t.closest('input, textarea, [contenteditable], .monaco-editor')) return
    e.preventDefault()
    searchRef.current?.focus()
  }

  // Narrow-mode drill-out: back to the list, guarding an unsaved draft.
  const goBack = () => {
    if (dirty && !window.confirm('Discard unsaved changes?')) return
    setSelected(null)
    setCreating(false)
    setLoaded(null)
    setDraft('')
    setLoadError(null)
    setSaveError(null)
    setDeleteError(null)
    setStaleOnDisk(false)
  }

  // ── split divider ───────────────────────────────────────────────────

  const draggingRef = useRef(false)
  const applyDividerAt = (clientX: number) => {
    const body = bodyRef.current
    if (!body) return
    const rect = body.getBoundingClientRect()
    if (rect.width <= 0) return
    setSplit(clampRatio((clientX - rect.left) / rect.width))
  }
  const persistSplit = useCallback((value: number) => writeStored(`${storageKey}:split`, String(value)), [storageKey])
  const onDividerPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    draggingRef.current = true
    setDragging(true)
  }
  const onDividerPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return
    applyDividerAt(e.clientX)
  }
  const endDividerDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return
    draggingRef.current = false
    setDragging(false)
    e.currentTarget.releasePointerCapture(e.pointerId)
    setSplit((v) => {
      persistSplit(v)
      return v
    })
  }
  const onDividerKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === 'ArrowLeft' ? -0.05 : e.key === 'ArrowRight' ? 0.05 : null
    if (step !== null) {
      e.preventDefault()
      setSplit((v) => {
        const next = clampRatio(v + step)
        persistSplit(next)
        return next
      })
    } else if (e.key === 'Enter') {
      e.preventDefault()
      setSplit(0.5)
      persistSplit(0.5)
    }
  }

  // ── derived view state ──────────────────────────────────────────────

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
  const showDoc = !narrow || selected !== null || creating

  const docKey = loaded?.key ?? selected
  const docSegments = docKey ? docKey.split('/') : []
  const docName = creating
    ? `new ${adapter.noun}`
    : (docSegments[docSegments.length - 1] ?? '')

  const modeOptions: EditorMode[] = narrow ? ['edit', 'preview'] : ['edit', 'split', 'preview']

  const row = rows?.find((item) => item.key === (loaded?.key || selected))
  const nameField = readFrontmatterField(
    draft,
    adapter.nameKeys ?? ['name'],
    adapter.defaultNameKey,
  )
  const descriptionField = readFrontmatterField(draft, ['description'])
  const nameValue = nameField.present
    ? nameField.value
    : creating
      ? ''
      : row?.title || row?.key || ''
  const descriptionValue = descriptionField.present
    ? descriptionField.value
    : creating
      ? ''
      : row?.description || ''
  const editorSource = withoutFrontmatterFields(draft, [
    nameField.key,
    descriptionField.key,
  ])
  const managedFields = [
    { ...nameField, value: nameValue, bare: adapter.slugName },
    { ...descriptionField, value: descriptionValue },
  ]

  const editDraft = (next: string) => {
    setDraft(next)
    setSaveError(null)
  }

  const draftLines = loaded === null ? 0 : draft.split('\n').length
  const draftBytes = loaded === null ? 0 : new Blob([draft]).size

  const saveStatus = saving ? 'saving…' : savedFlash ? '✓ saved' : dirty ? 'unsaved' : ''

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay ('/' jumps to the filter) around real controls
    <div
      className={`dir-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
      ref={rootRef}
      onKeyDown={onRootKeyDown}
    >
      {showSide ? (
        <aside className="dir-ui-side" aria-label={`${adapter.noun} list`}>
          <div className="dir-ui-side-top">
            {nav}
            {adapter.create ? (
              <Button
                variant="ghost"
                size="sm"
                className="dir-ui-new-btn"
                onClick={startCreate}
                aria-label={`new ${adapter.noun}`}
                title={`new ${adapter.noun}`}
              >
                <span className="dir-ui-new-mark">
                  <PlusIcon className="dir-ui-new-icon" />
                </span>
                <span>new {adapter.noun}</span>
              </Button>
            ) : null}
            <div className="dir-ui-search">
              <SearchIcon className="dir-ui-search-icon" />
              <Input
                ref={searchRef}
                value={search}
                onChange={setSearch}
                placeholder={`filter ${adapter.noun}s…`}
                aria-label={`filter ${adapter.noun}s`}
                className="dir-ui-search-input"
                onKeyDown={(e) => {
                  if (e.key === 'Escape' && search) {
                    e.stopPropagation()
                    setSearch('')
                  }
                }}
              />
              {search ? (
                <button
                  type="button"
                  className="dir-ui-search-clear"
                  aria-label="clear filter"
                  onClick={() => {
                    setSearch('')
                    searchRef.current?.focus()
                  }}
                >
                  <XIcon className="dir-ui-x-icon" />
                </button>
              ) : null}
            </div>
            {rows !== null && !listError ? (
              <div className="dir-ui-count" aria-live="polite">
                {needle
                  ? `${visible.length} of ${rows.length} ${adapter.noun}s`
                  : `${rows.length} ${adapter.noun}${rows.length === 1 ? '' : 's'}`}
              </div>
            ) : null}
          </div>

          <div className="dir-ui-side-scroll">
            {listError ? (
              <div className="dir-ui-error-panel">
                <p>
                  The {adapter.noun} list could not be loaded.
                  <span className="detail">{listError}</span>
                </p>
                <Button variant="ghost" size="sm" onClick={refreshList}>
                  retry
                </Button>
              </div>
            ) : rows === null ? (
              <div className="dir-ui-side-skel" aria-hidden>
                {[0, 1, 2, 3].map((i) => (
                  <div key={i} className="dir-ui-skel-row">
                    <span className="bar w60" />
                    <span className="bar w90" />
                    <span className="bar w40" />
                  </div>
                ))}
              </div>
            ) : visible.length === 0 ? (
              <div className="dir-ui-side-empty">
                {needle ? (
                  <>
                    <p>
                      No {adapter.noun}s match “{search.trim()}”.
                    </p>
                    <Button variant="ghost" size="sm" onClick={() => setSearch('')}>
                      clear filter
                    </Button>
                  </>
                ) : (
                  <p>No {adapter.noun}s yet.</p>
                )}
              </div>
            ) : (
              <ul className="dir-ui-nav-list">
                {visible.map((r) => {
                  const isTitled = r.title !== '' && r.title !== r.key
                  return (
                    <li key={r.key}>
                      <button
                        type="button"
                        className={`dir-ui-nav-row${r.key === selected ? ' active' : ''}`}
                        aria-current={r.key === selected ? 'true' : undefined}
                        onClick={() => open(r.key)}
                      >
                        <span className={`name${isTitled ? '' : ' mono'}`}>{isTitled ? r.title : r.key}</span>
                        {isTitled ? <span className="id">{r.key}</span> : null}
                        {r.description ? <span className="desc">{r.description}</span> : null}
                        <span className="fine">{r.fine}</span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>
        </aside>
      ) : null}

      {showDoc ? (
        <section className="dir-ui-doc" onKeyDown={onWorkspaceKeyDown} aria-label={`${adapter.noun} workspace`}>
          {selected === null && !creating ? (
            <div className="dir-ui-hero">
              <MarkdownFileIcon className="dir-ui-hero-icon" />
              <h2 className="dir-ui-hero-title">{adapter.emptyTitle}</h2>
              <p className="dir-ui-hero-body">{adapter.emptyBody}</p>
              <p className="dir-ui-hero-hint">
                <kbd>/</kbd> filters the list · <kbd>⌘S</kbd> saves
              </p>
            </div>
          ) : (
            <>
              <header className="dir-ui-doc-head">
                {narrow ? <BackButton onClick={goBack} label={`back to ${adapter.noun} list`} /> : null}
                <div className="dir-ui-doc-identity">
                  <span className="dir-ui-doc-name" title={docKey ?? ''}>
                    <span className="txt">{docName}</span>
                    {dirty ? <span className="dir-ui-dirty-dot" title="unsaved changes" aria-hidden /> : null}
                  </span>
                  {!narrow ? (
                    <span className="dir-ui-doc-crumb">{[adapter.crumbRoot, ...docSegments].join(' / ')}</span>
                  ) : null}
                </div>
                {/* biome-ignore lint/a11y/useSemanticElements: segmented control of buttons; fieldset chrome (min-content sizing) breaks the header row */}
                <div className="dir-ui-seg" role="group" aria-label="editor mode">
                  {modeOptions.map((m) => (
                    <button
                      key={m}
                      type="button"
                      className={`dir-ui-seg-btn${effMode === m ? ' active' : ''}`}
                      aria-pressed={effMode === m}
                      onClick={() => setMode(m)}
                    >
                      {m}
                    </button>
                  ))}
                </div>
                <div className="dir-ui-save-area">
                  {adapter.remove && !creating ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="dir-ui-delete-btn"
                      disabled={!loaded || saving || deleting}
                      onClick={remove}
                    >
                      {deleting ? 'deleting…' : 'delete'}
                    </Button>
                  ) : null}
                  <span className="dir-ui-save-note" aria-live="polite">
                    {saveStatus}
                  </span>
                  {dirty && !saving ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={deleting}
                      onClick={() => {
                        setDraft(loaded?.content ?? '')
                        setSaveError(null)
                      }}
                    >
                      revert
                    </Button>
                  ) : null}
                  <Button variant="primary" size="sm" disabled={!dirty || saving || deleting} onClick={save}>
                    {saving ? 'saving…' : 'save'}
                  </Button>
                </div>
              </header>

              {staleOnDisk ? (
                <div className="dir-ui-banner warn" role="status">
                  <span>This {adapter.noun} changed on disk while you were editing.</span>
                  <button
                    type="button"
                    className="dir-ui-linkish"
                    onClick={() => docKey && open(docKey, { reload: true })}
                  >
                    reload (discards your draft)
                  </button>
                </div>
              ) : null}

              {saveError ? (
                <div className="dir-ui-banner alert" role="alert">
                  <span>
                    Save failed.
                    <span className="detail">{saveError}</span>
                  </span>
                  <button type="button" className="dir-ui-linkish" onClick={save}>
                    retry
                  </button>
                  <button type="button" className="dir-ui-linkish quiet" onClick={() => setSaveError(null)}>
                    dismiss
                  </button>
                </div>
              ) : null}

              {deleteError ? (
                <div className="dir-ui-banner alert" role="alert">
                  <span>
                    Delete failed.
                    <span className="detail">{deleteError}</span>
                  </span>
                  <button type="button" className="dir-ui-linkish" onClick={remove}>
                    retry
                  </button>
                  <button type="button" className="dir-ui-linkish quiet" onClick={() => setDeleteError(null)}>
                    dismiss
                  </button>
                </div>
              ) : null}

              {loadError ? (
                <div className="dir-ui-error-panel grow">
                  <p>
                    {docKey} could not be loaded.
                    <span className="detail">{loadError}</span>
                  </p>
                  <Button variant="ghost" size="sm" onClick={() => docKey && open(docKey, { reload: true })}>
                    retry
                  </Button>
                </div>
              ) : loaded === null ? (
                <div className="dir-ui-doc-loading" aria-hidden>
                  <span className="bar w40" />
                  <span className="bar w90" />
                  <span className="bar w75" />
                  <span className="bar w85" />
                  <span className="bar w30" />
                </div>
              ) : (
                <>
                  <div className={`dir-ui-doc-body mode-${effMode}${dragging ? ' dragging' : ''}`} ref={bodyRef}>
                    <div className="dir-ui-pane editor" style={effMode === 'split' ? { flexGrow: split } : undefined}>
                      <div className="dir-ui-edit-fields">
                        <label className="dir-ui-edit-field" htmlFor={`${fieldId}-name`}>
                          <span className="dir-ui-edit-label">
                            name
                            {adapter.slugName ? (
                              <span className="dir-ui-edit-hint">a–z · 0–9 · - · _</span>
                            ) : null}
                          </span>
                          <Input
                            id={`${fieldId}-name`}
                            value={nameValue}
                            onChange={(next) =>
                              editDraft(
                                setFrontmatterField(
                                  draft,
                                  nameField.key,
                                  adapter.slugName ? next.toLowerCase() : next,
                                  adapter.slugName,
                                ),
                              )
                            }
                            placeholder={`${adapter.noun} name`}
                            preserveCase={!adapter.slugName}
                            required={adapter.slugName}
                            pattern={adapter.slugName ? '[a-z0-9_-]+' : undefined}
                            spellCheck={false}
                            className="dir-ui-edit-input"
                          />
                        </label>
                        <label
                          className="dir-ui-edit-field"
                          htmlFor={`${fieldId}-description`}
                        >
                          <span className="dir-ui-edit-label">description</span>
                          <textarea
                            id={`${fieldId}-description`}
                            value={descriptionValue}
                            onChange={(event) =>
                              editDraft(
                                setFrontmatterField(
                                  draft,
                                  descriptionField.key,
                                  event.currentTarget.value,
                                ),
                              )
                            }
                            placeholder={`what this ${adapter.noun} is for…`}
                            required={adapter.descriptionRequired}
                            rows={2}
                            className="dir-ui-edit-textarea"
                          />
                        </label>
                      </div>
                      <div className="dir-ui-source-head" aria-hidden>
                        <span>content</span>
                        <span>markdown</span>
                      </div>
                      <CodeEditor
                        value={editorSource}
                        onChange={(next) =>
                          editDraft(
                            restoreFrontmatterFields(next, managedFields),
                          )
                        }
                        language="markdown"
                        className="dir-ui-code"
                        aria-label={`${adapter.noun} markdown content`}
                        placeholder="# markdown…"
                      />
                    </div>
                    {effMode === 'split' ? (
                      // biome-ignore lint/a11y/useSemanticElements: drag handle is not an <hr>; semantic separator + tabIndex is enough
                      <div
                        className="dir-ui-divider"
                        role="separator"
                        aria-orientation="vertical"
                        aria-label="resize editor and preview"
                        aria-valuemin={25}
                        aria-valuemax={75}
                        aria-valuenow={Math.round(split * 100)}
                        tabIndex={0}
                        onPointerDown={onDividerPointerDown}
                        onPointerMove={onDividerPointerMove}
                        onPointerUp={endDividerDrag}
                        onPointerCancel={endDividerDrag}
                        onDoubleClick={() => {
                          setSplit(0.5)
                          persistSplit(0.5)
                        }}
                        onKeyDown={onDividerKeyDown}
                      />
                    ) : null}
                    {effMode !== 'edit' ? (
                      <div
                        className="dir-ui-pane preview"
                        style={effMode === 'split' ? { flexGrow: 1 - split } : undefined}
                      >
                        <div className="dir-ui-reading">
                          <MarkdownPreview markdown={frontmatterBody(draft)} className="dir-ui-preview" />
                        </div>
                      </div>
                    ) : null}
                  </div>
                  <footer className="dir-ui-statusbar">
                    <span className="fact">markdown</span>
                    <span className="fact">
                      {draftLines} line{draftLines === 1 ? '' : 's'}
                    </span>
                    <span className="fact">{formatKb(draftBytes)}</span>
                    <span className="spacer" />
                    <span className="fact">{dirty ? '⌘S saves' : 'all changes saved'}</span>
                  </footer>
                </>
              )}
            </>
          )}
        </section>
      ) : null}
    </div>
  )
}

function formatKb(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  return `${(bytes / 1024).toFixed(1)} KB`
}
