/**
 * The list + document browser skills and agent profiles are built on: a
 * library column (search, grouped entries, "new") beside one document —
 * read it, edit it, or both side by side — saved through the worker's
 * `*::update` functions from ONE floating save bar that every collection
 * shares (Revert · Save ⌘S, with the error and a retry in the same place).
 *
 * Live updates: the page observes the worker's `directory::*::on-change`
 * function from a tab-scoped handler (see `useOnChange`) — a download or
 * update landing anywhere refreshes the list, and reloads the open entry
 * when the editor isn't dirty (an unsaved draft is never clobbered; a
 * notice offers the reload instead).
 *
 * Layout adapts to the width the browser HAS (`useContainerNarrow`, not a
 * viewport media query). Under NARROW_BELOW px it becomes a drill-in flow:
 * the list fills the width, opening an entry swaps it for the document with
 * a ← back row, and side-by-side is unavailable.
 *
 * The editor stays MOUNTED across view switches (hidden while reading) so
 * Monaco keeps cursor and scroll; the split ratio and the view persist per
 * tab+collection (`usePaneState`).
 */

import {
  CodeEditor,
  ConfirmDialog,
  type Host,
  MarkdownPreview,
  type PageCommandsApi,
  PageSidebar,
  ShadcnBadge,
  ShadcnButton,
  ShadcnCheckbox,
  ShadcnKbd,
  ShadcnKbdGroup,
  ShadcnSkeleton,
  ShadcnTabs,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  ShadcnTextarea,
  ShadcnTooltip,
  ShadcnTooltipContent,
  ShadcnTooltipTrigger,
  useConfirm,
} from '@iii-dev/console-ui'
import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import { useContainerNarrow, usePaneState, useSplitDrag } from '@iii-dev/console-ui/hooks'
import { ArrowLeft, Check, Copy, Plus, Trash2 } from 'lucide-react'
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
import { Library } from './library'

/** Container width (px) below which the browser collapses to the
 * drill-in list ⇄ document flow. */
const NARROW_BELOW = 760

export interface BrowserRow {
  /** Stable key + the id/name passed to load/save. */
  key: string
  /** Optional leading glyph (agents: the tree-icon token). */
  icon?: ReactNode
  /** Agent color token, rendered as `data-color` on the glyph box. */
  iconTone?: string
  title: string
  description: string
  /** Fine-print line (size · modified), shown in the document header. */
  fine: string
  /** Library group id (skills: the first id segment; agents: the source). */
  group?: string
  /** Display label for `group` (defaults to the id). */
  groupLabel?: string
  /** A short tag at the end of the row title (`index`, `edited`…). */
  badge?: string
  /** Where the entry came from (agents: `kit acme/team`, `worker kanban`,
   * `local`…). */
  origin?: string
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
 * the built-in name/description fields entirely. Name and description stay
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
  /** Copy the open entry to `<id>-copy` and open the copy. */
  onDuplicate?: () => void
}

export interface BrowserAdapter {
  /** Singular noun for labels ("skill" / "agent profile"). */
  noun: string
  /** Plural, capitalised, for headings ("Skills" / "Agent profiles"). */
  nounPlural: string
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
  /** Error copy shown when the name fails `namePattern`. */
  nameHint?: string
  /** Helper copy under the create-time name field. */
  nameHelp?: string
  /** Where an id lands on disk (`skills/<id>.md`), previewed while creating. */
  filePath?: (id: string) => string
  /** Offer "Duplicate" (a create of the open file under `<id>-copy`). */
  duplicate?: boolean
  /** The entry's key is a slug SEPARATE from its free-text display name
      (agents: the id is the file stem, frontmatter `name` is display
      only). The id is derived from the name (slugified) at create time. */
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
  /** Full replacement for the built-in document (agents: the sectioned
      profile form). */
  customForm?: (ctx: FormContext) => ReactNode
  /** Shape-matched loading state for a custom form. */
  customLoading?: () => ReactNode
  /** The custom form edits the markdown body itself, so omit CodeEditor. */
  customFormOwnsContent?: boolean
  /** The custom form provides its own identity header, so omit the
      document header and the view switch. */
  customFormOwnsWorkspaceHeader?: boolean
  /** Show the skill's "manual only" (disable-model-invocation) control. */
  modelInvocationOption?: boolean
  /** Library group order (ids); the rest follow alphabetically. */
  groupOrder?: readonly string[]
  /** Label for rows without a group. */
  fallbackGroupLabel?: string
  /** Document empty state: what this collection is, in a sentence. */
  emptyTitle: string
  emptyBody: string
  /** Library empty state (the folder has nothing yet). */
  emptyListTitle: string
  emptyListBody: string
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
  /** The worker's change function to observe for external-change refreshes. */
  onChangeFunction: string
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

export type EditorMode = 'read' | 'edit' | 'split'

const MODES: { value: EditorMode; label: string }[] = [
  { value: 'read', label: 'Read' },
  { value: 'edit', label: 'Edit' },
  { value: 'split', label: 'Split' },
]

interface Loaded {
  key: string
  content: string
}

/** Keep the split ratio between a quarter and three quarters of the width. */
const clampRatio = (n: number) => Math.min(0.75, Math.max(0.25, n))

/** "Unable to save: <reason>." — the reason without a trailing stop. */
function saveFailure(e: unknown): string {
  return `Unable to save: ${errorMessage(e).replace(/[.\s]+$/, '')}.`
}

/** Tab-scoped observer of one of the worker's on-change functions. The
 * handler id carries the `iii::` prefix so per-event invocations stay
 * span-suppressed and out of the trace feed; the observer is GC'd with the
 * tab and disposed on unmount. */
function useOnChange(host: Host, functionId: string, onEvent: () => void) {
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent
  useEffect(() => {
    const slug = functionId.replace(/[^a-z0-9]+/g, '-')
    // `on` registers the handler tab-scoped as `<id>::<browserId>`; the
    // observer's target must name that full id (as the session chip does).
    const handlerId = `iii::iii-directory-ui::${slug}`
    const offHandler = host.iii.on(handlerId, () => {
      onEventRef.current()
    })
    const unobserve = host.iii.observe(functionId, `${handlerId}::${host.iii.browserId}`)
    return () => {
      unobserve()
      offHandler()
    }
  }, [host, functionId])
}

/** A shadcn tooltip around a trigger element. */
function Hint({ label, children }: { label: ReactNode; children: React.ReactElement }) {
  return (
    <ShadcnTooltip>
      <ShadcnTooltipTrigger render={children} />
      <ShadcnTooltipContent>{label}</ShadcnTooltipContent>
    </ShadcnTooltip>
  )
}

/** The list + document browser for one file collection (skills or agents): load, edit, save, delete, live updates. */
export function CollectionBrowser({
  host,
  adapter,
  nav,
  panelSide = 'left',
  storageKey,
  commands,
  active = true,
  pendingOpen,
  onCount,
}: {
  host: Host
  adapter: BrowserAdapter
  /** Sidebar top slot — the section switcher lives here. */
  nav?: ReactNode
  panelSide?: 'left' | 'right'
  /** localStorage namespace for per-tab+collection UI state. */
  storageKey: string
  /** The page's command surface — shared across every mounted collection. */
  commands?: PageCommandsApi
  /** Whether this collection is the one currently shown (all stay
      mounted); only the active one registers page-level commands/keys. */
  active?: boolean
  /** An external panel request targeted this collection — open an entry or
      start its creation flow. `id` is monotonic, so a repeated identical
      action still re-applies. */
  pendingOpen?: { id: number; key?: string; action?: 'create' } | null
  /** Reports the entry count whenever the list loads (the section tabs). */
  onCount?: (count: number) => void
}) {
  const [rows, setRows] = useState<BrowserRow[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [search, setSearch] = useState('')
  const [collapsedGroups, setCollapsedGroups] = usePaneState<string[]>(`${storageKey}:collapsed`, [])
  const searchRef = useRef<HTMLInputElement | null>(null)
  // The console's pane focus lands on `[data-autofocus]`; only the visible
  // collection may claim it (all stay mounted).
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

  const [storedMode, setMode] = usePaneState<EditorMode>(`${storageKey}:view`, 'read')
  const mode: EditorMode = MODES.some((m) => m.value === storedMode) ? storedMode : 'read'

  const [storedSplit, setSplit] = usePaneState<number>(`${storageKey}:split`, 0.5)
  const split = Number.isFinite(storedSplit) ? clampRatio(storedSplit) : 0.5
  const [dragging, setDragging] = useState(false)

  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)
  const [savedFlash, setSavedFlash] = useState(false)
  const [staleOnDisk, setStaleOnDisk] = useState(false)
  const [copied, setCopied] = useState(false)
  const [duplicating, setDuplicating] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const flashTimer = useRef<number | undefined>(undefined)
  const copyTimer = useRef<number | undefined>(undefined)
  useEffect(
    () => () => {
      window.clearTimeout(flashTimer.current)
      window.clearTimeout(copyTimer.current)
    },
    [],
  )

  const { ref: rootRef, narrow } = useContainerNarrow({ below: NARROW_BELOW })
  const bodyRef = useRef<HTMLDivElement | null>(null)

  const row = rows?.find((item) => item.key === (loaded?.key || selected))
  const readOnly = !creating && row?.readOnly === true
  const dirty = !readOnly && loaded !== null && draft !== loaded.content
  // A new entry is all fields: there is nothing to read yet. Side by side
  // needs room; narrow containers fall back to editing.
  const effMode: EditorMode = adapter.customFormOwnsWorkspaceHeader
    ? 'edit'
    : creating && mode === 'read'
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

  const onCountRef = useRef(onCount)
  onCountRef.current = onCount
  useEffect(() => {
    if (rows) onCountRef.current?.(rows.length)
  }, [rows])

  const dirtyRef = useRef(dirty)
  dirtyRef.current = dirty
  const { confirm, dialog } = useConfirm()
  const guardDirty = useCallback(
    (proceed: () => void) => {
      if (!dirtyRef.current) {
        proceed()
        return
      }
      void confirm({
        title: 'Discard unsaved changes?',
        description: 'Your edits will be lost.',
        confirmLabel: 'Discard changes',
        cancelLabel: 'Keep editing',
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

  const resetDocState = () => {
    setActionError(null)
    setLoadError(null)
    setSaveError(null)
    setStaleOnDisk(false)
  }

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
        resetDocState()
        adapter
          .load(host, key)
          .then((content) => {
            // Stale async result: another entry was opened meanwhile.
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
  const openRef = useRef(open)
  openRef.current = open

  /** Blank document for a new entry. Most collections count their scaffold
      as unsaved work; custom forms can treat it as a pristine baseline. */
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
      resetDocState()
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
  useOnChange(host, adapter.onChangeFunction, () => {
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
        setDraft((prev) => (selectedRef.current === key ? content : prev))
      })
      .catch(() => {
        /* entry may have been removed; the refreshed list reflects it */
      })
  })

  const flashSaved = () => {
    setSavedFlash(true)
    window.clearTimeout(flashTimer.current)
    flashTimer.current = window.setTimeout(() => setSavedFlash(false), 2000)
  }

  const save = useCallback(() => {
    if (readOnly || !loaded || saving || deleting || draft === loaded.content) return
    const name = readFrontmatterField(draft, adapter.nameKeys ?? ['name'], adapter.defaultNameKey).value.trim()
    if (adapter.nameRequired && !name) {
      setSaveError(`Give this ${adapter.noun} a name first.`)
      return
    }
    let effectiveName = name || (!creating ? loaded.key : '')
    if (adapter.separateId) {
      // The name is free-text display copy; the key is the slug id —
      // derived from the name at create time, fixed to the file stem after.
      effectiveName = creating ? slugify(name) : loaded.key
      if (creating && !adapter.separateId.pattern.test(effectiveName)) {
        setSaveError(adapter.separateId.hint)
        return
      }
    } else {
      // namePattern governs the CREATE id only: on update the frontmatter
      // name/title is a display field for skills (the id is the key).
      const pattern = creating ? adapter.namePattern : undefined
      if (pattern && !pattern.test(effectiveName)) {
        setSaveError(adapter.nameHint ?? 'Use lowercase letters, numbers, hyphens or underscores.')
        return
      }
    }
    if (creating) {
      if (!adapter.create) return
      setSaving(true)
      setSaveError(null)
      adapter
        .create(host, effectiveName, draft)
        .then((effectiveKey) => {
          refreshList()
          if (!creatingRef.current) return
          setCreating(false)
          setLoaded({ key: effectiveKey, content: draft })
          setSelected(effectiveKey)
          setStaleOnDisk(false)
          flashSaved()
        })
        .catch((e) => {
          if (creatingRef.current) setSaveError(saveFailure(e))
        })
        .finally(() => setSaving(false))
      return
    }
    const key = loaded.key
    setSaving(true)
    setSaveError(null)
    adapter
      .save(host, key, draft)
      .then((effectiveKey) => {
        // The write happened either way; only the editor state is stale if
        // the user navigated away mid-save.
        refreshList()
        if (selectedRef.current !== key) return
        setLoaded({ key: effectiveKey, content: draft })
        setSelected(effectiveKey)
        setStaleOnDisk(false)
        flashSaved()
      })
      .catch((e) => {
        if (selectedRef.current === key) setSaveError(saveFailure(e))
      })
      .finally(() => setSaving(false))
  }, [host, adapter, loaded, draft, saving, deleting, creating, readOnly, refreshList])

  // Duplicate: the open file (draft included) as a new entry under
  // `<id>-copy`, `-copy-2`… — whichever is free — then open the copy.
  const duplicate = useCallback(() => {
    const source = loaded?.key
    if (!adapter.duplicate || !adapter.create || !source || creating || duplicating) return
    const taken = new Set((rows ?? []).map((r) => r.key))
    let id = `${source}-copy`
    for (let n = 2; taken.has(id); n += 1) id = `${source}-copy-${n}`
    const name = readFrontmatterField(draft, adapter.nameKeys ?? ['name'], adapter.defaultNameKey)
    const content =
      name.present && name.value.trim() ? setFrontmatterField(draft, name.key, `${name.value.trim()} copy`) : draft
    const create = adapter.create
    guardDirty(() => {
      setDuplicating(true)
      setActionError(null)
      create(host, id, content)
        .then((key) => {
          refreshList()
          openRef.current(key, { reload: true })
        })
        .catch((e) => setActionError(`Unable to duplicate: ${errorMessage(e).replace(/[.\s]+$/, '')}.`))
        .finally(() => setDuplicating(false))
    })
  }, [adapter, host, loaded, draft, rows, creating, duplicating, guardDirty, refreshList])

  const revert = useCallback(() => {
    setDraft(loaded?.content ?? '')
    setSaveError(null)
  }, [loaded])

  const removeNow = useCallback(
    (key: string) => {
      if (!adapter.remove) return Promise.resolve()
      setDeleting(true)
      setSaveError(null)
      return adapter
        .remove(host, key)
        .then(() => {
          setRows((current) => current?.filter((r) => r.key !== key) ?? null)
          refreshList()
          if (selectedRef.current !== key) return
          setSelected(null)
          setLoaded(null)
          setDraft('')
          setLoadError(null)
          setStaleOnDisk(false)
        })
        .finally(() => setDeleting(false))
    },
    [host, adapter, refreshList],
  )

  const canDelete = !!adapter.remove && !creating && !readOnly && row?.noDelete !== true

  // Deleting asks in the console's ConfirmDialog; its async onConfirm keeps
  // the dialog open (busy) until the file is gone, and shows a failure there
  // with a retry instead of a banner somewhere else.
  const [pendingDelete, setPendingDelete] = useState<string | null>(null)
  const remove = useCallback(() => {
    if (!canDelete || !loaded || saving || deleting) return
    setPendingDelete(loaded.key)
  }, [loaded, saving, deleting, canDelete])

  const copyId = useCallback(() => {
    const key = loaded?.key || selected
    if (!key) return
    void copyText(key).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.clearTimeout(copyTimer.current)
      copyTimer.current = window.setTimeout(() => setCopied(false), 1400)
    })
  }, [loaded, selected])

  const onWorkspaceKeyDown = (e: React.KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
      e.preventDefault()
      save()
    }
  }

  // `/` anywhere outside a text surface jumps to the library search.
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
      resetDocState()
    })
  }

  // The collection's primary verbs, for the palette and the keyboard while
  // this pane has focus — only the visible collection registers.
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
        title: `Search ${adapter.nounPlural.toLowerCase()}`,
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

  const divider = useSplitDrag<{ x: number; left: number; width: number }>({
    horizontal: true,
    begin: (e) => {
      const rect = bodyRef.current?.getBoundingClientRect()
      if (!rect || rect.width <= 0) return null
      setDragging(true)
      return { x: e.clientX, left: rect.left, width: rect.width }
    },
    move: (o, delta) => setSplit(clampRatio((o.x + delta - o.left) / o.width)),
    step: (dir) => setSplit((v) => clampRatio(v + dir * 0.05)),
  })
  const endDividerDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    divider.onPointerUp(e)
    setDragging(false)
  }
  const onDividerKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    divider.onKeyDown(e)
    if (!e.defaultPrevented && e.key === 'Enter') {
      e.preventDefault()
      setSplit(0.5)
    }
  }

  // ── derived view state ──────────────────────────────────────────────

  const { showSide, showDoc } = resolveBrowserPaneVisibility({ narrow, selected, creating })

  const docKey = loaded?.key || selected
  const nameField = readFrontmatterField(draft, adapter.nameKeys ?? ['name'], adapter.defaultNameKey)
  const descriptionField = readFrontmatterField(draft, ['description'])
  const nameValue = nameField.present ? nameField.value : creating ? '' : row?.title || row?.key || ''
  const descriptionValue = descriptionField.present ? descriptionField.value : creating ? '' : row?.description || ''
  const modelInvocationField = readFrontmatterField(draft, ['disable-model-invocation'])
  const modelInvocationSimple = frontmatterFieldIsSimpleBoolean(draft, 'disable-model-invocation')
  const manualOnly = /:\s*(?:true|True|TRUE)\s*$/.test(modelInvocationField.raw ?? '')
  const typeField = readFrontmatterField(draft, ['type'])
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

  const displayTitle = creating ? `New ${adapter.noun}` : nameValue.trim() || docKey || ''
  const createPath =
    creating && adapter.filePath && !adapter.separateId && nameValue.trim() ? adapter.filePath(nameValue.trim()) : null

  const backRow = narrow ? (
    <div className="dir-doc-back">
      <ShadcnButton variant="ghost" size="sm" onClick={goBack}>
        <ArrowLeft aria-hidden />
        {adapter.nounPlural}
      </ShadcnButton>
    </div>
  ) : null

  // ── the document ────────────────────────────────────────────────────

  let doc: ReactNode
  if (selected === null && !creating) {
    doc = (
      <div className="dir-doc-empty">
        <div className="dir-doc-empty-card">
          <h2 className="dir-doc-empty-title">{adapter.emptyTitle}</h2>
          <p className="dir-doc-empty-body">{adapter.emptyBody}</p>
          {adapter.create ? (
            <div className="dir-doc-empty-actions">
              <ShadcnButton size="sm" onClick={startCreate}>
                <Plus aria-hidden />
                New {adapter.noun}
              </ShadcnButton>
            </div>
          ) : null}
        </div>
      </div>
    )
  } else if (loadError) {
    doc = (
      <>
        {backRow}
        <div className="dir-doc-empty">
          <div className="dir-doc-empty-card" role="alert">
            <h2 className="dir-doc-empty-title">Unable to open this {adapter.noun}</h2>
            <p className="dir-doc-empty-body">{loadError}</p>
            <div className="dir-doc-empty-actions">
              <ShadcnButton variant="secondary" size="sm" onClick={() => docKey && open(docKey, { reload: true })}>
                Try again
              </ShadcnButton>
            </div>
          </div>
        </div>
      </>
    )
  } else if (adapter.customForm) {
    doc = (
      <>
        {backRow}
        {staleNotice()}
        <div className="dir-doc-scroll dir-doc-custom">
          {loaded === null
            ? (adapter.customLoading?.() ?? <DocSkeleton />)
            : adapter.customForm({
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
                onRemove: canDelete ? remove : undefined,
                onDuplicate: adapter.duplicate && adapter.create && !creating && !duplicating ? duplicate : undefined,
              })}
        </div>
      </>
    )
  } else {
    const segments = docKey ? docKey.split('/') : []
    // Editing an existing entry reads like the document itself: the name is
    // the title and the description the lead, typed into in place (no
    // boxes). A new entry is a small labelled form (its id needs the rules).
    const inPlace = !creating && effMode !== 'read'
    const manualOnlyControl =
      adapter.modelInvocationOption && modelInvocationSimple ? (
        <label className="dir-check" htmlFor={`${fieldId}-manual`}>
          <ShadcnCheckbox
            id={`${fieldId}-manual`}
            checked={manualOnly}
            disabled={readOnly}
            onCheckedChange={(checked) =>
              editDraft(
                checked
                  ? setFrontmatterField(draft, 'disable-model-invocation', 'true', true)
                  : withoutFrontmatterFields(draft, ['disable-model-invocation']),
              )
            }
          />
          <span className="dir-check-text">
            <span className="dir-check-label">Manual only</span>
            <span className="dir-check-help">Agents only use this {adapter.noun} when someone names it.</span>
          </span>
        </label>
      ) : null
    doc = (
      <>
        {backRow}
        <header className="dir-doc-head">
          <div className="dir-doc-identity">
            {inPlace ? null : (
              <h2 className="dir-doc-title">
                <span className="dir-doc-title-text">{displayTitle}</span>
                {readOnly ? <ShadcnBadge variant="outline">Read-only</ShadcnBadge> : null}
              </h2>
            )}
            <div className="dir-doc-meta">
              {creating ? (
                <span>Not saved yet</span>
              ) : (
                <>
                  <Hint label={copied ? 'Copied' : 'Copy id'}>
                    <button type="button" className="dir-doc-id" onClick={copyId}>
                      <span className="dir-mono">{segments.join(' / ')}</span>
                      <CopyGlyph copied={copied} />
                      <span className="dir-sr-only">{copied ? 'Copied' : 'Copy id'}</span>
                    </button>
                  </Hint>
                  {typeField.value.trim() ? <span className="dir-doc-chip">{typeField.value.trim()}</span> : null}
                  {manualOnly ? <span className="dir-doc-chip">Manual only</span> : null}
                  {row ? <span className="dir-doc-fine">{row.fine}</span> : null}
                </>
              )}
            </div>
          </div>
          <div className="dir-doc-actions">
            <ModeSwitch value={effMode} narrow={narrow} creating={creating} onChange={setMode} />
          </div>
        </header>
        {staleNotice()}
        {loaded === null ? (
          <DocSkeleton />
        ) : (
          <div className={`dir-doc-body mode-${effMode}${dragging ? ' dragging' : ''}`} ref={bodyRef}>
            <div className="dir-pane dir-pane-edit" style={effMode === 'split' ? { flexGrow: split } : undefined}>
              {inPlace ? (
                <div className="dir-fields dir-fields-inplace">
                  <input
                    id={`${fieldId}-name`}
                    className="dir-inplace-title"
                    value={nameValue}
                    onChange={(e) => editDraft(setFrontmatterField(draft, nameField.key, e.currentTarget.value))}
                    placeholder="Untitled"
                    aria-label="Name"
                    spellCheck={false}
                    autoComplete="off"
                    readOnly={readOnly}
                  />
                  <textarea
                    id={`${fieldId}-description`}
                    className="dir-inplace-lead"
                    value={descriptionValue}
                    onChange={(e) => editDraft(setFrontmatterField(draft, descriptionField.key, e.currentTarget.value))}
                    placeholder="Add a description: use when the user asks to …"
                    aria-label="Description"
                    readOnly={readOnly}
                    rows={1}
                  />
                  {manualOnlyControl || canDelete ? (
                    <div className="dir-fields-foot">
                      {manualOnlyControl ?? <span />}
                      {canDelete ? (
                        <ShadcnButton
                          variant="ghost"
                          size="sm"
                          className="dir-delete"
                          disabled={saving || deleting}
                          onClick={remove}
                        >
                          <Trash2 aria-hidden />
                          Delete {adapter.noun}
                        </ShadcnButton>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              ) : (
                <div className="dir-fields">
                  <label className="dir-field" htmlFor={`${fieldId}-name`}>
                    <span className="dir-field-label">{creating && !adapter.separateId ? 'Id' : 'Name'}</span>
                    <input
                      id={`${fieldId}-name`}
                      className={creating && !adapter.separateId ? 'dir-input dir-mono' : 'dir-input'}
                      value={nameValue}
                      onChange={(e) => editDraft(setFrontmatterField(draft, nameField.key, e.currentTarget.value))}
                      placeholder={creating && !adapter.separateId ? 'team/release-notes' : 'Release notes'}
                      required={adapter.nameRequired}
                      spellCheck={false}
                      autoComplete="off"
                      readOnly={readOnly}
                      aria-describedby={creating ? `${fieldId}-name-help` : undefined}
                      // biome-ignore lint/a11y/noAutofocus: a new entry starts at its name
                      autoFocus={creating}
                    />
                    {creating ? (
                      <span className="dir-field-help" id={`${fieldId}-name-help`}>
                        {adapter.separateId
                          ? slugify(nameValue)
                            ? `Saved as ${slugify(nameValue)}.md`
                            : 'The file name comes from the name.'
                          : adapter.nameHelp}
                        {createPath ? (
                          <>
                            {' '}
                            Saved as <span className="dir-mono">{createPath}</span>
                          </>
                        ) : null}
                      </span>
                    ) : null}
                  </label>
                  <label className="dir-field" htmlFor={`${fieldId}-description`}>
                    <span className="dir-field-label">Description</span>
                    <ShadcnTextarea
                      id={`${fieldId}-description`}
                      className="dir-textarea"
                      value={descriptionValue}
                      onChange={(e) =>
                        editDraft(setFrontmatterField(draft, descriptionField.key, e.currentTarget.value))
                      }
                      placeholder="Use when the user asks to …"
                      readOnly={readOnly}
                      rows={2}
                    />
                  </label>
                  {manualOnlyControl}
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
                <div className="dir-source">
                  <div className="dir-source-head">
                    <span id={`${fieldId}-instructions`}>Instructions</span>
                    <span className="dir-source-meta">Markdown</span>
                  </div>
                  <CodeEditor
                    value={editorSource}
                    onChange={(next) => editDraft(restoreFrontmatterFields(next, managedFields))}
                    language="markdown"
                    fill
                    fontSize={14}
                    readOnly={readOnly}
                    className="dir-code"
                    aria-label="Instructions"
                    placeholder="# Write the instructions in markdown"
                  />
                </div>
              ) : null}
            </div>
            {effMode === 'split' ? (
              // biome-ignore lint/a11y/useSemanticElements: drag handle is not an <hr>; semantic separator + tabIndex is enough
              <div
                className="dir-divider"
                role="separator"
                aria-orientation="vertical"
                aria-label="Resize editor and preview"
                aria-valuemin={25}
                aria-valuemax={75}
                aria-valuenow={Math.round(split * 100)}
                tabIndex={0}
                onPointerDown={divider.onPointerDown}
                onPointerMove={divider.onPointerMove}
                onPointerUp={endDividerDrag}
                onPointerCancel={endDividerDrag}
                onLostPointerCapture={endDividerDrag}
                onDoubleClick={() => setSplit(0.5)}
                onKeyDown={onDividerKeyDown}
              />
            ) : null}
            {effMode !== 'edit' ? (
              <div className="dir-pane dir-pane-read" style={effMode === 'split' ? { flexGrow: 1 - split } : undefined}>
                <article className="dir-reading">
                  {effMode === 'read' && descriptionValue ? (
                    <p className="dir-reading-lead">{descriptionValue}</p>
                  ) : null}
                  {frontmatterBody(draft).trim() ? (
                    <MarkdownPreview markdown={frontmatterBody(draft)} className="dir-ui-preview" />
                  ) : (
                    <p className="dir-reading-empty">No instructions yet.</p>
                  )}
                </article>
              </div>
            ) : null}
          </div>
        )}
      </>
    )
  }

  function staleNotice() {
    if (actionError) {
      return (
        <div className="dir-notice" role="alert" data-tone="alert">
          <span>{actionError}</span>
          <span className="dir-notice-actions">
            <ShadcnButton variant="ghost" size="sm" onClick={() => setActionError(null)}>
              Dismiss
            </ShadcnButton>
          </span>
        </div>
      )
    }
    if (!staleOnDisk) return null
    return (
      <div className="dir-notice" role="status" data-tone="warn">
        <span>This {adapter.noun} changed on disk while you were editing.</span>
        <span className="dir-notice-actions">
          <ShadcnButton variant="secondary" size="sm" onClick={() => docKey && open(docKey, { reload: true })}>
            Reload from disk
          </ShadcnButton>
          <ShadcnButton variant="ghost" size="sm" onClick={() => setStaleOnDisk(false)}>
            Keep my edits
          </ShadcnButton>
        </span>
      </div>
    )
  }

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: shortcut relay ('/' jumps to search) around real controls
    <div
      className={`dir-browser${narrow ? ' narrow' : ''}${panelSide === 'right' ? ' right' : ''}`}
      ref={rootRef}
      onKeyDown={onRootKeyDown}
    >
      {dialog}
      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(next) => {
          if (!next) setPendingDelete(null)
        }}
        title={`Delete ${adapter.noun}?`}
        description={`This ${adapter.noun}’s file will be permanently deleted. This cannot be undone.`}
        confirmLabel={`Delete ${adapter.noun}`}
        busyLabel="Deleting…"
        tone="danger"
        onConfirm={() => (pendingDelete ? removeNow(pendingDelete).then(() => setPendingDelete(null)) : undefined)}
      />
      {showSide ? (
        <PageSidebar
          label={`${adapter.noun} list`}
          side={panelSide}
          collapsible
          storageKey="iii-directory:library"
          defaultWidth={320}
          minWidth={260}
          narrow={narrow}
          className="dir-side"
          header={nav}
          collapsedActions={
            adapter.create ? (
              <ShadcnButton variant="ghost" size="icon-sm" aria-label={`New ${adapter.noun}`} onClick={startCreate}>
                <Plus aria-hidden />
              </ShadcnButton>
            ) : undefined
          }
        >
          <Library
            noun={adapter.noun}
            nounPlural={adapter.nounPlural}
            rows={rows}
            error={listError}
            onRetry={refreshList}
            selected={creating ? null : selected}
            onOpen={(key) => open(key)}
            query={search}
            onQuery={setSearch}
            searchRef={attachSearch}
            onCreate={adapter.create ? startCreate : undefined}
            groupOrder={adapter.groupOrder}
            fallbackGroupLabel={adapter.fallbackGroupLabel}
            collapsed={collapsedGroups}
            onToggleGroup={(id) =>
              setCollapsedGroups((current) =>
                current.includes(id) ? current.filter((g) => g !== id) : [...current, id],
              )
            }
            emptyTitle={adapter.emptyListTitle}
            emptyBody={adapter.emptyListBody}
          />
        </PageSidebar>
      ) : null}

      {showDoc ? (
        <section className="dir-doc" onKeyDown={onWorkspaceKeyDown} aria-label={`${adapter.noun} document`}>
          {doc}
          <SaveBar
            visible={!readOnly && (dirty || saving || savedFlash || !!saveError)}
            creating={creating}
            noun={adapter.noun}
            dirty={dirty}
            saving={saving}
            saved={savedFlash}
            error={saveError}
            busy={deleting}
            onRevert={creating ? undefined : revert}
            onSave={save}
          />
        </section>
      ) : null}
    </div>
  )
}

/** Read · Edit · Split as labelled segments. Tabs, so ←/→ move between
 * them and the selection is announced; the active segment carries a ring
 * as well as its ink, never colour alone. A new entry has nothing to read,
 * and narrow panes have no room to split. */
function ModeSwitch({
  value,
  narrow,
  creating,
  onChange,
}: {
  value: EditorMode
  narrow: boolean
  creating: boolean
  onChange: (next: EditorMode) => void
}) {
  const options = MODES.filter((m) => !(narrow && m.value === 'split') && !(creating && m.value === 'read'))
  if (options.length < 2) return null
  return (
    <ShadcnTabs value={value} onValueChange={(next) => onChange(next as EditorMode)} className="dir-modes-root">
      <ShadcnTabsList activateOnFocus className="dir-modes" aria-label="View">
        {options.map(({ value: v, label }) => (
          <ShadcnTabsTrigger key={v} value={v} className="dir-mode">
            {label}
          </ShadcnTabsTrigger>
        ))}
      </ShadcnTabsList>
    </ShadcnTabs>
  )
}

/** Copy ⇄ copied: both glyphs stay mounted and cross-fade (scale, opacity,
 * blur), so the swap reads as one icon changing its mind. */
export function CopyGlyph({ copied }: { copied: boolean }) {
  return (
    <span className="dir-copy-glyph" data-copied={copied ? '' : undefined} aria-hidden>
      <Copy className="dir-copy-glyph-idle" />
      <Check className="dir-copy-glyph-done" />
    </span>
  )
}

/** The one save affordance for every collection. It enters once when the
 * draft turns dirty (never per keystroke) and says what is happening —
 * unsaved, saving, saved, failed — in the same place. */
export function SaveBar({
  visible,
  creating,
  noun,
  dirty,
  saving,
  saved,
  error,
  busy,
  onRevert,
  onSave,
}: {
  visible: boolean
  creating: boolean
  noun: string
  dirty: boolean
  saving: boolean
  saved: boolean
  error: string | null
  busy: boolean
  onRevert?: () => void
  onSave: () => void
}) {
  const status = error
    ? error
    : saving
      ? 'Saving…'
      : saved && !dirty
        ? 'Saved'
        : creating
          ? `New ${noun}, not saved yet`
          : 'Unsaved changes'
  return (
    <div className="dir-savebar" data-visible={visible ? '' : undefined} aria-hidden={!visible} inert={!visible}>
      <div className="dir-savebar-card" role={error ? 'alert' : 'status'} data-tone={error ? 'alert' : undefined}>
        <span className="dir-savebar-status">
          {saved && !dirty && !error ? <Check aria-hidden /> : null}
          <span className="dir-savebar-text">{status}</span>
        </span>
        {dirty || error ? (
          <span className="dir-savebar-actions">
            {onRevert && dirty ? (
              <ShadcnButton variant="ghost" size="sm" disabled={saving || busy} onClick={onRevert}>
                Revert
              </ShadcnButton>
            ) : null}
            <ShadcnButton size="sm" disabled={!dirty || saving || busy} onClick={onSave}>
              {error ? 'Try again' : creating ? `Create ${noun}` : 'Save'}
              <ShadcnKbdGroup className="dir-savebar-kbd" aria-hidden>
                <ShadcnKbd>⌘</ShadcnKbd>
                <ShadcnKbd>S</ShadcnKbd>
              </ShadcnKbdGroup>
            </ShadcnButton>
          </span>
        ) : null}
      </div>
    </div>
  )
}

/** Loading placeholder shaped like a document header and a few lines of body. */
function DocSkeleton() {
  return (
    <div className="dir-doc-skeleton" aria-hidden>
      <ShadcnSkeleton style={{ width: '38%', height: 22 }} />
      <ShadcnSkeleton style={{ width: '22%', height: 12 }} />
      <div className="dir-doc-skeleton-gap" />
      {['92%', '84%', '88%', '60%', '90%', '72%'].map((w, i) => (
        <ShadcnSkeleton key={i} style={{ width: w, height: 12 }} />
      ))}
    </div>
  )
}

/** Which panes show: both when wide; in narrow panes the list until an entry (or a new one) is open, then the document. */
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
