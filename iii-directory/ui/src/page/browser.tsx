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
 * Layout adapts to the width the browser HAS (`useContainerNarrow`, not a
 * viewport media query — the console can host it in panes of any size).
 * Under NARROW_BELOW px it becomes a drill-in flow: the list fills the
 * width, opening an entry swaps the list for the document with a ← back
 * button, and split mode is unavailable (edit/preview only).
 *
 * The editor pane stays MOUNTED across mode switches (hidden in preview
 * mode) so Monaco keeps cursor and scroll position; the split divider is
 * draggable and its ratio, like the view mode, persists per tab+collection
 * (`usePaneState`).
 */

import {
  Button,
  CodeEditor,
  EmptyState,
  Eyebrow,
  type Host,
  IconButton,
  Input,
  Kbd,
  KeyCombo,
  List,
  ListItem,
  MarkdownPreview,
  type PageCommandsApi,
  PageSidebar,
  SearchField,
  SegmentedControl,
  Skeleton,
  StatusBar,
  StatusDot,
  StatusPanel,
  useConfirm,
} from '@iii-dev/console-ui'
import { errorMessage, formatBytes } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState } from '@iii-dev/console-ui/hooks'
import uiClasses from '@iii-dev/console-ui/ui-classes'
import { ChevronLeft, FileText, Plus } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { draftAction, parseStoredDraft, type StoredDraft } from './draft-storage'
import {
  frontmatterBody,
  frontmatterFieldIsSimpleBoolean,
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
  /** Optional leading glyph (agents: the tree-icon token). */
  icon?: ReactNode
  /** Agent color token, rendered as `data-color` on the glyph box. */
  iconTone?: string
  title: string
  description: string
  /** Fine-print line (size · modified). */
  fine: string
  /** Built-in entries can be viewed and copied but not changed. */
  readOnly?: boolean
  /** Editable entries with no backing file yet (saving creates one) have
   * nothing to delete. */
  noDelete?: boolean
}

/** What an adapter's `extraFields` renderer gets to work with: the full
 * draft document plus the same guarded editor the built-in fields use. */
export interface ExtraFieldsContext {
  host: Host
  draft: string
  editDraft: (next: string) => void
  readOnly: boolean
  fieldId: string
  /** The opened entry's key; null while creating a new one. */
  entryKey: string | null
}

/** Context for a full custom form (`adapter.customForm`), which replaces
 * the built-in name/description grid entirely. Name and description stay
 * managed frontmatter fields — the setters here write them the same way
 * the built-in inputs would. */
export interface FormContext extends ExtraFieldsContext {
  nameValue: string
  descriptionValue: string
  setName: (next: string) => void
  setDescription: (next: string) => void
  creating: boolean
  dirty: boolean
  saving: boolean
  saved: boolean
  deleting: boolean
  onSave: () => void
  onRemove?: () => void
}

export interface BrowserAdapter {
  /** Singular noun for labels ("skill" / "agent profile"). */
  noun: string
  /** Breadcrumb root segment ("skills" / "agents"). */
  crumbRoot: string
  /** Frontmatter keys that may carry the displayed name, in precedence
      order. Skills support both legacy `title` and standard `name`. */
  nameKeys?: readonly string[]
  /** Field inserted when the document has no declared name yet. */
  defaultNameKey?: string
  /** Grammar the name must match when CREATING — skills allow
      slash-separated segments. Unset: any name saves. */
  namePattern?: RegExp
  /** Error copy shown when the name fails `namePattern` (falls back to a
      generic slug message). */
  nameHint?: string
  /** The entry's key is a slug SEPARATE from its free-text display name
      (agents: the id is the file stem, frontmatter `name` is display
      only). The name field stays free text; the id is derived from it
      (slugified) at create time and shown as a hint, never typed. */
  separateId?: { pattern: RegExp; hint: string }
  /** The scanner rejects an empty display name (agents). */
  nameRequired?: boolean
  /** Starter document for a new entry (defaults to name+description). */
  newTemplate?: string
  /** Treat the starter document as the pristine create-form baseline. */
  newTemplateStartsClean?: boolean
  /** Extra frontmatter keys managed by `extraFields` — hidden from the
      content editor and restored verbatim on save, like name/description. */
  extraManagedKeys?: readonly string[]
  /** Adapter-specific form controls rendered under the built-in fields. */
  extraFields?: (ctx: ExtraFieldsContext) => ReactNode
  /** Full replacement for the built-in fields block (agents: the
      sectioned Identity / Behavior / Execution form). Wins over
      `extraFields`. */
  customForm?: (ctx: FormContext) => ReactNode
  /** Shape-matched loading state for a custom form. */
  customLoading?: () => ReactNode
  /** The custom form edits the markdown body itself, so omit CodeEditor. */
  customFormOwnsContent?: boolean
  /** The custom form provides its own identity/actions, so omit the generic
      title, mode tabs, and save toolbar. */
  customFormOwnsWorkspaceHeader?: boolean
  /** Render list entries as identity rows with a larger glyph and title. */
  prominentListItems?: boolean
  /** Label for the source editor header (default "Content"). */
  sourceLabel?: string
  /** Show the skill's model-invocation control. */
  modelInvocationOption?: boolean
  /** Workspace empty-state copy. */
  emptyTitle: string
  emptyBody: string
  list(host: Host): Promise<BrowserRow[]>
  /** Load one entry's full on-disk content (frontmatter included). */
  load(host: Host, key: string): Promise<string>
  /** Save; returns the entry's effective key after the write (a rename in
      frontmatter moves the selection along). */
  save(host: Host, key: string, content: string): Promise<string>
  /** Create a NEW entry; returns its key. Omit to hide the "new" button. */
  create?(host: Host, name: string, content: string): Promise<string>
  /** Permanently remove an existing entry. Omit when deletion is unsupported. */
  remove?(host: Host, key: string): Promise<void>
  /** Worker trigger type to subscribe for external-change refreshes. */
  onChangeType: string
}

/** Starter frontmatter for a new entry: scaffold the keys the editor's own
 *  fields bind to rather than opening a blank page. */
const NEW_TEMPLATE = '---\nname: \ndescription: ""\n---\n\n'

/** Derive a slug id from a free-text display name ("Release Captain" →
 * "release-captain"), used to prefill the id field while creating. */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

type EditorMode = 'edit' | 'split' | 'preview'

const EDITOR_MODE_LABELS: Record<EditorMode, string> = {
  edit: 'Edit',
  split: 'Split',
  preview: 'Preview',
}

interface Loaded {
  key: string
  content: string
}

const clampRatio = (n: number) => Math.min(0.75, Math.max(0.25, n))

/** Tab-scoped subscription to one of the worker's on-change trigger
 * types. The handler id carries the `iii::` prefix so per-event
 * invocations stay span-suppressed and out of the trace feed; the
 * binding is GC'd with the tab and unregistered on unmount. (Not
 * `useWorkerLive`: the handler needs the event itself to apply the
 * dirty-draft guard and the optimistic list edits, not a refetched cache.) */
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

function SkeletonLines({ widths, className }: { widths: string[]; className?: string }) {
  return (
    <div className={className} aria-hidden>
      {widths.map((width, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static placeholder lines
        <Skeleton key={i} style={{ width }} />
      ))}
    </div>
  )
}

export function CollectionBrowser({
  host,
  adapter,
  nav,
  panelSide = 'left',
  storageKey,
  commands,
  active = true,
  pendingOpen,
}: {
  host: Host
  adapter: BrowserAdapter
  /** Sidebar top slot — the collection switcher lives here. */
  nav?: ReactNode
  panelSide?: 'left' | 'right'
  /** localStorage namespace for per-tab+collection UI state. */
  storageKey: string
  /** The page's command surface — shared across every mounted collection. */
  commands?: PageCommandsApi
  /** Whether this collection is the one currently shown (both stay
      mounted); only the active one registers page-level commands/keys. */
  active?: boolean
  /** An external panel request targeted this collection — open an entry or
      start its creation flow. `id` is monotonic, so a repeated identical
      action still re-applies. */
  pendingOpen?: { id: number; key?: string; action?: 'create' } | null
}) {
  const [rows, setRows] = useState<BrowserRow[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const searchRef = useRef<HTMLInputElement | null>(null)
  // The console's pane focus lands on `[data-autofocus]`; only the visible
  // collection may claim it (both stay mounted).
  const attachSearch = useCallback(
    (el: HTMLInputElement | null) => {
      searchRef.current = el
      el?.toggleAttribute('data-autofocus', active)
    },
    [active],
  )
  const fieldId = useId()

  /* Restore unsaved work from the last unmount (tab switch). A creating
     draft is self-contained; a selected-entry draft still needs the disk
     baseline, loaded by the mount effect below. */
  const [storedDraft, setStoredDraft] = usePaneState<StoredDraft | null>(`${storageKey}:draft`, null)
  const [restored] = useState(() => parseStoredDraft(storedDraft))
  const [selected, setSelected] = useState<string | null>(restored && !restored.creating ? restored.key : null)
  const [creating, setCreating] = useState(restored?.creating ?? false)
  const [loaded, setLoaded] = useState<Loaded | null>(restored?.creating ? { key: '', content: '' } : null)
  const [draft, setDraft] = useState(restored?.content ?? '')
  const [loadError, setLoadError] = useState<string | null>(null)

  const [storedMode, setMode] = usePaneState<EditorMode>(`${storageKey}:mode`, 'split')
  const mode: EditorMode = storedMode in EDITOR_MODE_LABELS ? storedMode : 'split'

  const [storedSplit, setSplit] = usePaneState<number>(`${storageKey}:split`, 0.5)
  const split = Number.isFinite(storedSplit) ? clampRatio(storedSplit) : 0.5
  const [dragging, setDragging] = useState(false)

  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [savedFlash, setSavedFlash] = useState(false)
  const [staleOnDisk, setStaleOnDisk] = useState(false)
  const flashTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(flashTimer.current), [])

  const { ref: rootRef, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const bodyRef = useRef<HTMLDivElement | null>(null)

  const row = rows?.find((item) => item.key === (loaded?.key || selected))
  const readOnly = !creating && row?.readOnly === true
  const dirty = !readOnly && loaded !== null && draft !== loaded.content
  // Split needs side-by-side room; narrow containers fall back to edit.
  const effMode: EditorMode = adapter.customFormOwnsWorkspaceHeader
    ? 'edit'
    : narrow && mode === 'split'
      ? 'edit'
      : mode

  const refreshList = useCallback(() => {
    adapter
      .list(host)
      .then((next) => {
        setRows(next)
        setListError(null)
      })
      .catch((e) => setListError(errorMessage(e)))
  }, [host, adapter])

  useEffect(() => {
    refreshList()
  }, [refreshList])

  const dirtyRef = useRef(dirty)
  dirtyRef.current = dirty
  // A dirty draft asks in the console's own dialog, never window.confirm:
  // the native box blocks the whole tab and cannot say what is at stake
  // when the draft is a new, unnamed entry.
  const { confirm, dialog } = useConfirm()
  const guardDirty = useCallback(
    (proceed: () => void) => {
      if (!dirtyRef.current) {
        proceed()
        return
      }
      const label = selectedRef.current ?? 'this new entry'
      void confirm({
        title: 'Discard unsaved changes?',
        description: `The unsaved changes to ${label} will be lost.`,
        confirmLabel: 'Discard',
        tone: 'danger',
      }).then((ok) => {
        if (ok) proceed()
      })
    },
    [confirm],
  )
  const selectedRef = useRef(selected)
  selectedRef.current = selected
  const creatingRef = useRef(creating)
  creatingRef.current = creating

  /* One-shot: fetch the disk baseline under a restored selected-entry draft
     so dirty tracking and save() have the real `loaded.content` to diff
     against. The restored draft itself is kept, never overwritten. */
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only restore
  useEffect(() => {
    if (!restored || restored.creating || !restored.key) return
    const key = restored.key
    adapter
      .load(host, key)
      .then((content) => {
        if (selectedRef.current !== key) return
        setLoaded({ key, content })
      })
      .catch((e) => {
        if (selectedRef.current === key) setLoadError(errorMessage(e))
      })
  }, [])

  /* Mirror unsaved work to pane state on every change, so a tab switch
     (which unmounts this page) can restore it. See `draftAction`. */
  useEffect(() => {
    const action = draftAction({
      creating,
      selected,
      draft,
      loadedContent: loaded?.content ?? null,
    })
    if (action.kind === 'write') setStoredDraft(action.draft)
    else if (action.kind === 'clear') setStoredDraft(null)
  }, [creating, selected, draft, loaded, setStoredDraft])

  const open = useCallback(
    (key: string, opts?: { reload?: boolean }) => {
      // Discard confirmed (or a reload): the persisted draft dies now, not
      // when the load lands — an unmount in between must not resurrect it.
      const proceed = () => {
        setStoredDraft(null)
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
            if (selectedRef.current === key) setLoadError(errorMessage(e))
          })
      }
      if (opts?.reload) {
        proceed()
        return
      }
      // Re-clicking the open entry must not clobber the draft; switching
      // away from an unsaved draft asks first.
      if (key === selectedRef.current) return
      guardDirty(proceed)
    },
    [host, adapter, setStoredDraft, guardDirty],
  )

  /** Blank editor for a new entry. Most collections count their scaffold as
      unsaved work; custom forms can treat it as a pristine visual baseline. */
  const startCreate = useCallback(() => {
    guardDirty(() => {
      const template = adapter.newTemplate ?? NEW_TEMPLATE
      setStoredDraft(null)
      setCreating(true)
      setSelected(null)
      setLoaded({
        key: '',
        content: adapter.newTemplateStartsClean ? template : '',
      })
      setDraft(template)
      setLoadError(null)
      setSaveError(null)
      setDeleteError(null)
      setStaleOnDisk(false)
    })
  }, [setStoredDraft, guardDirty, adapter])

  const appliedOpenRef = useRef(0)
  useEffect(() => {
    if (!pendingOpen || pendingOpen.id === appliedOpenRef.current) return
    appliedOpenRef.current = pendingOpen.id
    if (pendingOpen.action === 'create') {
      startCreate()
    } else if (pendingOpen.key) {
      open(pendingOpen.key)
    }
  }, [pendingOpen, open, startCreate])

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
    if (readOnly || !loaded || saving || deleting || draft === loaded.content) return
    const name = readFrontmatterField(draft, adapter.nameKeys ?? ['name'], adapter.defaultNameKey).value.trim()
    if (adapter.nameRequired && !name) {
      setSaveError(`enter a display name for this ${adapter.noun}`)
      return
    }
    let effectiveName = name || (!creating ? loaded.key : '')
    if (adapter.separateId) {
      // The name is free-text display copy; the key is the slug id —
      // derived from the name at create time, fixed to the file stem
      // afterwards. A collision surfaces as the server's conflict error.
      effectiveName = creating ? slugify(name) : loaded.key
      if (creating && !adapter.separateId.pattern.test(effectiveName)) {
        setSaveError(adapter.separateId.hint)
        return
      }
    } else {
      // namePattern governs the CREATE id only: on update the frontmatter
      // name/title is a display field for skills (the id is the key), so a
      // human-readable title must not block saving.
      const pattern = creating ? adapter.namePattern : undefined
      if (pattern && !pattern.test(effectiveName)) {
        setSaveError(adapter.nameHint ?? 'enter a name using lowercase letters, numbers, hyphens or underscores')
        return
      }
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
          flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2400)
        })
        .catch((e) => {
          if (creatingRef.current) setSaveError(errorMessage(e))
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
        if (selectedRef.current === key) setSaveError(errorMessage(e))
      })
      .finally(() => setSaving(false))
  }, [host, adapter, loaded, draft, saving, deleting, creating, readOnly, refreshList])

  const revert = useCallback(() => {
    setDraft(loaded?.content ?? '')
    setSaveError(null)
  }, [loaded])

  const removeNow = useCallback(
    (key: string) => {
      if (!adapter.remove) return
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
          if (selectedRef.current === key) setDeleteError(errorMessage(error))
        })
        .finally(() => setDeleting(false))
    },
    [host, adapter, refreshList],
  )

  const remove = useCallback(() => {
    if (readOnly || row?.noDelete === true) return
    if (!adapter.remove || !loaded || creating || saving || deleting) return
    const key = loaded.key
    const dirtyNote = dirty ? ' Your unsaved changes will also be lost.' : ''
    void confirm({
      title: `Delete ${adapter.noun} “${key}”?`,
      description: `This removes its file and cannot be undone.${dirtyNote}`,
      confirmLabel: 'Delete',
      tone: 'danger',
    }).then((ok) => {
      if (ok) removeNow(key)
    })
  }, [adapter, loaded, creating, saving, deleting, readOnly, row?.noDelete, dirty, removeNow, confirm])

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
    guardDirty(() => {
      setStoredDraft(null)
      setSelected(null)
      setCreating(false)
      setLoaded(null)
      setDraft('')
      setLoadError(null)
      setSaveError(null)
      setDeleteError(null)
      setStaleOnDisk(false)
    })
  }

  // The collection's primary verbs, for the palette and for the keyboard
  // while this pane has the focus — only the visible collection registers,
  // since both stay mounted at once. The `/` and ⌘S raw handlers above
  // keep working as-is; these just make the same actions ⌘K-visible.
  useEffect(() => {
    if (!active) return
    return commands?.register([
      {
        id: 'new-entry',
        title: `New ${adapter.noun}`,
        enabled: () => !!adapter.create,
        run: startCreate,
      },
      {
        id: 'filter',
        title: `Filter ${adapter.noun}s`,
        run: () => searchRef.current?.focus(),
      },
      {
        id: 'save',
        title: 'Save',
        shortcut: 'Mod+S',
        firesWhileTyping: true,
        enabled: () => dirty && !saving && !deleting,
        run: save,
      },
    ])
  }, [commands, active, adapter, startCreate, save, dirty, saving, deleting])

  // ── split divider ───────────────────────────────────────────────────

  const draggingRef = useRef(false)
  const applyDividerAt = (clientX: number) => {
    const body = bodyRef.current
    if (!body) return
    const rect = body.getBoundingClientRect()
    if (rect.width <= 0) return
    setSplit(clampRatio((clientX - rect.left) / rect.width))
  }
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
  }
  const onDividerKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === 'ArrowLeft' ? -0.05 : e.key === 'ArrowRight' ? 0.05 : null
    if (step !== null) {
      e.preventDefault()
      setSplit((v) => clampRatio(v + step))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      setSplit(0.5)
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

  // Narrow: one pane at a time — the list, an existing entry, or the
  // creation form. Creation intentionally has no selected key yet.
  const { showSide, showDoc } = resolveBrowserPaneVisibility({
    narrow,
    selected,
    creating,
  })

  const docKey = loaded?.key ?? selected
  const docSegments = docKey ? docKey.split('/') : []
  const docName = creating ? `New ${adapter.noun}` : (docSegments[docSegments.length - 1] ?? '')
  const nounPlural = `${adapter.noun[0].toUpperCase()}${adapter.noun.slice(1)}s`

  const modeOptions: EditorMode[] = narrow ? ['edit', 'preview'] : ['edit', 'split', 'preview']

  const nameField = readFrontmatterField(draft, adapter.nameKeys ?? ['name'], adapter.defaultNameKey)
  const descriptionField = readFrontmatterField(draft, ['description'])
  const nameValue = nameField.present ? nameField.value : creating ? '' : row?.title || row?.key || ''
  const descriptionValue = descriptionField.present ? descriptionField.value : creating ? '' : row?.description || ''
  const modelInvocationField = readFrontmatterField(draft, ['disable-model-invocation'])
  const modelInvocationSimple = frontmatterFieldIsSimpleBoolean(draft, 'disable-model-invocation')
  const modelInvocationDisabled = /:\s*(?:true|True|TRUE)\s*$/.test(modelInvocationField.raw ?? '')
  const managedFields = [
    { ...nameField, value: nameValue },
    { ...descriptionField, value: descriptionValue },
    ...(adapter.modelInvocationOption && modelInvocationSimple ? [{ ...modelInvocationField, bare: true }] : []),
    // Extra keys an adapter's custom controls own (agents: logo, skills):
    // hidden from the content editor, restored verbatim from `raw` on save.
    ...(adapter.extraManagedKeys ?? []).map((key) => readFrontmatterField(draft, [key])),
  ]
  const editorSource = withoutFrontmatterFields(
    draft,
    managedFields.map((field) => field.key),
  )

  const editDraft = (next: string) => {
    if (readOnly) return
    setDraft(next)
    setSaveError(null)
  }

  const draftLines = loaded === null ? 0 : draft.split('\n').length
  const draftBytes = loaded === null ? 0 : new Blob([draft]).size

  const saveStatus = saving ? 'Saving…' : savedFlash ? '✓ Saved' : dirty ? 'Unsaved' : ''

  const backButton = (
    <IconButton label={`back to ${adapter.noun} list`} onClick={goBack}>
      <ChevronLeft />
    </IconButton>
  )

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay ('/' jumps to the filter) around real controls
    <div
      className={`dir-ui-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}${adapter.prominentListItems ? ' prominent-list' : ''}`}
      ref={rootRef}
      onKeyDown={onRootKeyDown}
    >
      {dialog}
      {showSide ? (
        <PageSidebar
          label={`${adapter.noun} list`}
          side={panelSide}
          collapsible
          storageKey="iii-directory:navigation"
          defaultWidth={300}
          narrow={narrow}
          className="dir-ui-side"
          header={nav}
          collapsedActions={
            adapter.create ? (
              <IconButton label={`new ${adapter.noun}`} onClick={startCreate}>
                <Plus />
              </IconButton>
            ) : undefined
          }
        >
          <div className="dir-ui-side-top">
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
                  <Plus />
                </span>
                <span>New {adapter.noun}</span>
              </Button>
            ) : null}
            <SearchField
              ref={attachSearch}
              value={search}
              onChange={setSearch}
              placeholder={`filter ${adapter.noun}s…`}
              aria-label={`filter ${adapter.noun}s`}
            />
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
              <div className="dir-ui-error">
                <StatusPanel
                  variant="alert"
                  headline={`The ${adapter.noun} list could not be loaded.`}
                  detail={listError}
                />
                <Button variant="ghost" size="sm" onClick={refreshList}>
                  Retry
                </Button>
              </div>
            ) : rows === null ? (
              <div className="dir-ui-skel" aria-hidden>
                {[0, 1, 2, 3].map((i) => (
                  <SkeletonLines key={i} className="dir-ui-skel-row" widths={['60%', '90%', '40%']} />
                ))}
              </div>
            ) : visible.length === 0 ? (
              <div className="dir-ui-side-empty">
                {needle ? (
                  <EmptyState
                    title={`No ${adapter.noun}s match`}
                    description={`Nothing here matches “${search.trim()}”.`}
                    action={{ label: 'Clear filter', onClick: () => setSearch('') }}
                  />
                ) : (
                  <EmptyState title={`No ${adapter.noun}s yet`} description={`Nothing in the ${adapter.crumbRoot} folder.`} />
                )}
              </div>
            ) : (
              <List>
                {visible.map((r) => {
                  const isTitled = r.title !== '' && r.title !== r.key
                  const current = r.key === selected
                  return (
                    <ListItem
                      key={r.key}
                      className={`dir-ui-nav-row${current ? ' active' : ''}`}
                      aria-current={current ? 'true' : undefined}
                      onClick={() => open(r.key)}
                      leading={
                        r.icon ? (
                          <span className="dir-ui-nav-ico" data-color={r.iconTone} aria-hidden>
                            {r.icon}
                          </span>
                        ) : undefined
                      }
                      label={<span className={isTitled ? undefined : 'dir-ui-mono'}>{isTitled ? r.title : r.key}</span>}
                      description={
                        adapter.prominentListItems ? (
                          r.description || r.key
                        ) : (
                          <>
                            {isTitled ? <span className="dir-ui-nav-id">{r.key}</span> : null}
                            {r.description ? <span className="dir-ui-nav-desc">{r.description}</span> : null}
                            <span className="dir-ui-nav-fine">{r.fine}</span>
                          </>
                        )
                      }
                    />
                  )
                })}
              </List>
            )}
          </div>
        </PageSidebar>
      ) : null}

      {showDoc ? (
        <section className="dir-ui-doc" onKeyDown={onWorkspaceKeyDown} aria-label={`${adapter.noun} workspace`}>
          {selected === null && !creating ? (
            <div className="dir-ui-hero">
              <EmptyState icon={FileText} title={adapter.emptyTitle} description={adapter.emptyBody} />
              <p className="dir-ui-hero-hint">
                <Kbd>/</Kbd> filters the list · <KeyCombo binding="Mod+S" /> saves
              </p>
            </div>
          ) : (
            <>
              {!adapter.customFormOwnsWorkspaceHeader ? (
                <header className="dir-ui-doc-head">
                  {narrow ? backButton : null}
                  <div className="dir-ui-doc-identity">
                    <span className="dir-ui-doc-name" title={docKey ?? ''}>
                      <span className="dir-ui-doc-name-text">{docName}</span>
                      {dirty ? <StatusDot tone="accent" title="unsaved changes" /> : null}
                    </span>
                    {!narrow ? (
                      <span className="dir-ui-doc-crumb">{[adapter.crumbRoot, ...docSegments].join(' / ')}</span>
                    ) : null}
                  </div>
                  <SegmentedControl<EditorMode>
                    value={effMode}
                    onChange={setMode}
                    options={modeOptions.map((mode) => ({
                      value: mode,
                      label: EDITOR_MODE_LABELS[mode],
                    }))}
                    className="dir-ui-editor-tabs"
                    aria-label="Editor mode"
                  />
                  <div className="dir-ui-save-area">
                    {adapter.remove && !creating && !readOnly && row?.noDelete !== true ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="dir-ui-delete-btn"
                        disabled={!loaded || saving || deleting}
                        onClick={remove}
                      >
                        {deleting ? 'Deleting…' : 'Delete'}
                      </Button>
                    ) : null}
                    <span className="dir-ui-save-note" aria-live="polite">
                      {readOnly ? 'Read only' : saveStatus}
                    </span>
                    {dirty && !saving ? (
                      <Button variant="ghost" size="sm" disabled={deleting} onClick={revert}>
                        Revert
                      </Button>
                    ) : null}
                    {!readOnly ? (
                      <Button variant="primary" size="sm" disabled={!dirty || saving || deleting} onClick={save}>
                        {saving ? 'Saving…' : 'Save'}
                      </Button>
                    ) : null}
                  </div>
                </header>
              ) : null}

              {adapter.customFormOwnsWorkspaceHeader && (narrow || (dirty && !readOnly)) ? (
                <div className="dir-ui-af-save-panel">
                  {narrow ? (
                    <div className="dir-ui-doc-mobile-back">
                      {backButton}
                      <span>{nounPlural}</span>
                    </div>
                  ) : null}
                  {dirty && !readOnly ? (
                    <div className="dir-ui-af-save-actions">
                      <Button variant="ghost" size="sm" disabled={saving || deleting} onClick={revert}>
                        Revert
                      </Button>
                      <Button variant="primary" size="sm" disabled={saving || deleting} onClick={save}>
                        {saving ? 'Saving…' : 'Save'}
                      </Button>
                    </div>
                  ) : null}
                </div>
              ) : null}

              {staleOnDisk ? (
                <div className="dir-ui-banner" role="status">
                  <StatusPanel
                    variant="warn"
                    headline={`This ${adapter.noun} changed on disk while you were editing.`}
                    detail={
                      <Button variant="ghost" size="sm" onClick={() => docKey && open(docKey, { reload: true })}>
                        Reload (discards your draft)
                      </Button>
                    }
                  />
                </div>
              ) : null}

              {saveError ? (
                <div className="dir-ui-banner" role="alert">
                  <StatusPanel
                    variant="alert"
                    headline="Save failed."
                    detail={
                      <>
                        <span className="dir-ui-banner-detail">{saveError}</span>
                        <Button variant="ghost" size="sm" onClick={save}>
                          Retry
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setSaveError(null)}>
                          Dismiss
                        </Button>
                      </>
                    }
                  />
                </div>
              ) : null}

              {deleteError ? (
                <div className="dir-ui-banner" role="alert">
                  <StatusPanel
                    variant="alert"
                    headline="Delete failed."
                    detail={
                      <>
                        <span className="dir-ui-banner-detail">{deleteError}</span>
                        <Button variant="ghost" size="sm" onClick={remove}>
                          Retry
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setDeleteError(null)}>
                          Dismiss
                        </Button>
                      </>
                    }
                  />
                </div>
              ) : null}

              {loadError ? (
                <div className="dir-ui-error grow">
                  <StatusPanel variant="alert" headline={`${docKey} could not be loaded.`} detail={loadError} />
                  <Button variant="ghost" size="sm" onClick={() => docKey && open(docKey, { reload: true })}>
                    Retry
                  </Button>
                </div>
              ) : loaded === null ? (
                adapter.customLoading ? (
                  adapter.customLoading()
                ) : (
                  <SkeletonLines className="dir-ui-doc-loading" widths={['40%', '90%', '75%', '85%', '30%']} />
                )
              ) : (
                <>
                  <div className={`dir-ui-doc-body mode-${effMode}${dragging ? ' dragging' : ''}`} ref={bodyRef}>
                    <div className="dir-ui-pane editor" style={effMode === 'split' ? { flexGrow: split } : undefined}>
                      {adapter.customForm ? (
                        adapter.customForm({
                          host,
                          draft,
                          editDraft,
                          readOnly,
                          fieldId,
                          entryKey: creating ? null : (loaded?.key ?? selected),
                          nameValue,
                          descriptionValue,
                          setName: (next) => editDraft(setFrontmatterField(draft, nameField.key, next)),
                          setDescription: (next) => editDraft(setFrontmatterField(draft, descriptionField.key, next)),
                          creating,
                          dirty,
                          saving,
                          saved: savedFlash,
                          deleting,
                          onSave: save,
                          onRemove:
                            adapter.remove && !creating && !readOnly && row?.noDelete !== true
                              ? remove
                              : undefined,
                        })
                      ) : (
                        <div className="dir-ui-edit-fields">
                          <label className="dir-ui-edit-field" htmlFor={`${fieldId}-name`}>
                            <Eyebrow className="dir-ui-edit-label">
                              name
                              {adapter.separateId && creating ? (
                                <span className="dir-ui-edit-hint">
                                  {slugify(nameValue) ? `→ ${slugify(nameValue)}.md` : 'the id derives from the name'}
                                </span>
                              ) : null}
                            </Eyebrow>
                            <Input
                              id={`${fieldId}-name`}
                              value={nameValue}
                              onChange={(next) => editDraft(setFrontmatterField(draft, nameField.key, next))}
                              placeholder={`${adapter.noun} name`}
                              required={adapter.nameRequired}
                              spellCheck={false}
                              readOnly={readOnly}
                            />
                          </label>
                          <label className="dir-ui-edit-field" htmlFor={`${fieldId}-description`}>
                            <Eyebrow className="dir-ui-edit-label">Description</Eyebrow>
                            <textarea
                              id={`${fieldId}-description`}
                              value={descriptionValue}
                              onChange={(event) =>
                                editDraft(setFrontmatterField(draft, descriptionField.key, event.currentTarget.value))
                              }
                              placeholder={`what this ${adapter.noun} is for…`}
                              readOnly={readOnly}
                              rows={2}
                              className="dir-ui-edit-textarea"
                            />
                          </label>
                          {adapter.modelInvocationOption ? (
                            modelInvocationSimple ? (
                              <label className="dir-ui-checkrow dir-ui-model-invocation">
                                <input
                                  id={`${fieldId}-disable-model-invocation`}
                                  type="checkbox"
                                  checked={modelInvocationDisabled}
                                  disabled={readOnly}
                                  onChange={(event) =>
                                    editDraft(
                                      event.currentTarget.checked
                                        ? setFrontmatterField(draft, 'disable-model-invocation', 'true', true)
                                        : withoutFrontmatterFields(draft, ['disable-model-invocation']),
                                    )
                                  }
                                />
                                <span className={uiClasses.field}>
                                  <span className={uiClasses.fieldLabel}>Require explicit invocation</span>
                                  <span className={uiClasses.fieldDescription}>
                                    The model won’t select this skill automatically.
                                  </span>
                                </span>
                              </label>
                            ) : (
                              <div className={`${uiClasses.field} dir-ui-model-invocation`}>
                                <span className={uiClasses.fieldLabel}>Model invocation</span>
                                <span className={uiClasses.fieldDescription}>Advanced value — edit it in Content.</span>
                              </div>
                            )
                          ) : null}
                          {adapter.extraFields?.({
                            host,
                            draft,
                            editDraft,
                            readOnly,
                            fieldId,
                            entryKey: creating ? null : (loaded?.key ?? selected),
                          })}
                        </div>
                      )}
                      {!adapter.customFormOwnsContent ? (
                        <>
                          <div className="dir-ui-source-head" aria-hidden>
                            <Eyebrow>{adapter.sourceLabel ?? 'Content'}</Eyebrow>
                            <Eyebrow>Markdown</Eyebrow>
                          </div>
                          <CodeEditor
                            value={editorSource}
                            onChange={(next) => editDraft(restoreFrontmatterFields(next, managedFields))}
                            language="markdown"
                            readOnly={readOnly}
                            className="dir-ui-code"
                            aria-label={`${adapter.noun} markdown content`}
                            placeholder="# markdown…"
                          />
                        </>
                      ) : null}
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
                        onDoubleClick={() => setSplit(0.5)}
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
                  <StatusBar
                    className="dir-ui-statusbar"
                    end={<span>{readOnly ? 'read only' : dirty ? '⌘S saves' : 'all changes saved'}</span>}
                  >
                    <span>Markdown</span>
                    <span>
                      {draftLines} line{draftLines === 1 ? '' : 's'}
                    </span>
                    <span>{formatBytes(draftBytes)}</span>
                  </StatusBar>
                </>
              )}
            </>
          )}
        </section>
      ) : null}
    </div>
  )
}

export function resolveBrowserPaneVisibility({
  narrow,
  selected,
  creating,
}: {
  narrow: boolean
  selected: string | null
  creating: boolean
}): { showSide: boolean; showDoc: boolean } {
  return {
    showSide: !narrow || (selected === null && !creating),
    showDoc: !narrow || selected !== null || creating,
  }
}
