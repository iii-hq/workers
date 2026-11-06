import {
  CodeEditor,
  type Host,
  IconButton,
  type ModelOption,
  ModelPicker,
  Select,
  ShadcnButton,
  ShadcnDropdownMenu,
  ShadcnDropdownMenuContent,
  ShadcnDropdownMenuGroup,
  ShadcnDropdownMenuItem,
  ShadcnDropdownMenuTrigger,
  ShadcnTabs,
  ShadcnTabsContent,
  ShadcnTabsList,
  ShadcnTabsTrigger,
  Skeleton,
  Switch,
} from '@iii-dev/console-ui'
import { copyText } from '@iii-dev/console-ui/format'
import {
  Bot,
  Check,
  ClipboardCheck,
  CodeXml,
  Copy,
  CopyPlus,
  Database,
  Ellipsis,
  FileText,
  FlaskConical,
  type LucideIcon,
  Palette,
  Search,
  Terminal,
  Trash2,
  X,
} from 'lucide-react'
import type { KeyboardEvent, ReactNode } from 'react'
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { type FormContext, slugify } from './browser'
import {
  frontmatterBody,
  readFrontmatterField,
  readFrontmatterStringList,
  setFrontmatterBody,
  setFrontmatterField,
  setFrontmatterStringList,
  withoutFrontmatterFields,
} from './frontmatter'
import { countLabel } from './library-model'

/** The nine harness `SubagentIcon` glyphs — the same Lucide icons the
 * console's session tree renders for each token (ActiveSubagentChips). */
const TOKEN_ICONS: Record<string, LucideIcon> = {
  agent: Bot,
  code: CodeXml,
  search: Search,
  terminal: Terminal,
  database: Database,
  test: FlaskConical,
  review: ClipboardCheck,
  docs: FileText,
  design: Palette,
}

export function TokenIcon({ token, className }: { token: string; className?: string }) {
  const Icon = TOKEN_ICONS[token]
  return Icon ? <Icon aria-hidden className={className} /> : null
}

interface PickItem {
  id: string
  label: string
  desc?: string
}

interface CatalogState<T> {
  items: T[] | null
  error: boolean
}

/** Last catalog per host + fetcher: reopening an agent paints at once from
 * it (no skeleton, no reveal) while a fresh copy loads behind. */
const catalogCache = new WeakMap<Host, Map<unknown, unknown[]>>()

function useCatalog<T>(host: Host, fetch: (host: Host) => Promise<T[]>): CatalogState<T> {
  const [items, setItems] = useState<T[] | null>(() => (catalogCache.get(host)?.get(fetch) as T[] | undefined) ?? null)
  const [error, setError] = useState(false)

  useEffect(() => {
    let cancelled = false
    setError(false)
    fetch(host)
      .then((next) => {
        let byFetch = catalogCache.get(host)
        if (!byFetch) {
          byFetch = new Map()
          catalogCache.set(host, byFetch)
        }
        byFetch.set(fetch, next)
        if (!cancelled) setItems(next)
      })
      .catch(() => {
        if (!cancelled) setError(true)
      })
    return () => {
      cancelled = true
    }
  }, [host, fetch])

  return { items, error }
}

const fetchSkills = (host: Host) =>
  host.iii
    .call<{
      skills: { id: string; title: string; description: string }[]
    }>('directory::skills::list', { include_description: true })
    .then((out) =>
      (out.skills ?? []).map((skill) => ({
        id: skill.id,
        label: skill.title && skill.title !== skill.id ? skill.title : skill.id,
        desc: skill.description || undefined,
      })),
    )

/** One row of `engine::functions::list` — a first-party engine contract, so
 * the fields are trusted like `directory::skills::list`'s are in `fetchSkills`. */
interface EngineFunctionRow {
  function_id: string
  description?: string | null
}

/** The live function registry — what a profile's preloaded functions are
 * picked from. Ids are the labels: they ARE the contract the model calls. */
const fetchFunctions = (host: Host): Promise<PickItem[]> =>
  host.iii.call<{ functions?: EngineFunctionRow[] }>('engine::functions::list', {}).then((out) =>
    (out.functions ?? [])
      .flatMap((row) => {
        const id = row.function_id.trim()
        return id ? [{ id, label: id, desc: row.description?.trim() || undefined }] : []
      })
      .sort((a, b) => a.id.localeCompare(b.id)),
  )

interface AgentCatalogRow {
  id: string
  name: string
  builtin?: boolean
  inheritance_error?: string | null
}

const fetchAgents = (host: Host): Promise<AgentCatalogRow[]> =>
  host.iii.call<{ agents?: AgentCatalogRow[] }>('directory::agents::list').then((out) => out.agents ?? [])

interface CatalogModelRow {
  id?: unknown
  provider?: unknown
  display_name?: unknown
  context_window?: unknown
  supports_thinking?: unknown
  supports_vision?: unknown
  reasoning_efforts?: unknown
}

function parseReasoningEfforts(value: unknown) {
  if (!Array.isArray(value)) return undefined
  const efforts = value.flatMap((raw) => {
    if (!raw || typeof raw !== 'object') return []
    const row = raw as Record<string, unknown>
    if (typeof row.effort !== 'string' || !row.effort.trim()) return []
    return [
      {
        effort: row.effort.trim(),
        description: typeof row.description === 'string' && row.description.trim() ? row.description.trim() : undefined,
      },
    ]
  })
  return efforts.length > 0 ? efforts : undefined
}

const fetchModels = (host: Host): Promise<ModelOption[]> =>
  host.iii.call<{ models?: CatalogModelRow[] }>('router::models::list', {}).then((out) =>
    (out.models ?? []).flatMap((row) => {
      const id = typeof row.id === 'string' ? row.id.trim() : ''
      const provider = typeof row.provider === 'string' ? row.provider.trim() : ''
      if (!id) return []
      const catalogId = provider && !id.includes('::') ? `${provider}::${id}` : id
      return [
        {
          id: catalogId,
          label: typeof row.display_name === 'string' && row.display_name.trim() ? row.display_name.trim() : id,
          contextWindow: typeof row.context_window === 'number' ? row.context_window : undefined,
          supportsThinking: typeof row.supports_thinking === 'boolean' ? row.supports_thinking : undefined,
          supportsVision: typeof row.supports_vision === 'boolean' ? row.supports_vision : undefined,
          reasoningEfforts: parseReasoningEfforts(row.reasoning_efforts),
        },
      ]
    }),
  )

const LOGO_PRESETS: { emoji: string; token: string }[] = [
  { emoji: '🤖', token: 'agent' },
  { emoji: '💻', token: 'code' },
  { emoji: '🔍', token: 'search' },
  { emoji: '📟', token: 'terminal' },
  { emoji: '💾', token: 'database' },
  { emoji: '🧪', token: 'test' },
  { emoji: '🧐', token: 'review' },
  { emoji: '📚', token: 'docs' },
  { emoji: '🎨', token: 'design' },
]

const AGENT_COLORS = [
  { id: 'neutral', label: 'Neutral' },
  { id: 'blue', label: 'Blue' },
  { id: 'purple', label: 'Purple' },
  { id: 'teal', label: 'Teal' },
  { id: 'green', label: 'Green' },
  { id: 'amber', label: 'Amber' },
  { id: 'rose', label: 'Rose' },
] as const

type AgentColor = (typeof AGENT_COLORS)[number]['id']

function agentColor(value: string): AgentColor {
  return AGENT_COLORS.some((color) => color.id === value) ? (value as AgentColor) : 'neutral'
}

function AvatarPicker({
  icon,
  color,
  readOnly,
  onIconChange,
  onColorChange,
}: {
  icon: string
  color: string
  readOnly: boolean
  onIconChange: (preset: { emoji: string; token: string } | null) => void
  onColorChange: (color: AgentColor) => void
}) {
  const [state, setState] = useState<'closed' | 'open' | 'closing'>('closed')
  const wrapRef = useRef<HTMLDivElement>(null)
  const popRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  const close = useCallback(() => setState((current) => (current === 'open' ? 'closing' : current)), [])

  // The popover unmounts when its exit animation ends; with no animation
  // (reduced motion) it goes at once — Radix Presence's rule.
  useEffect(() => {
    if (state !== 'closing') return
    if (popRef.current && getComputedStyle(popRef.current).animationName === 'none') setState('closed')
  }, [state])

  useEffect(() => {
    if (state !== 'open') return
    const onPointerDown = (event: PointerEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) close()
    }
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      close()
      triggerRef.current?.focus()
    }
    document.addEventListener('pointerdown', onPointerDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [close, state])

  const selectedColor = agentColor(color)

  return (
    <div ref={wrapRef} className="dir-ui-af-avatar-wrap">
      <button
        ref={triggerRef}
        type="button"
        className="dir-ui-af-avatar-trigger"
        data-color={selectedColor}
        aria-label="Choose agent avatar"
        aria-haspopup="dialog"
        aria-expanded={state === 'open'}
        disabled={readOnly}
        onClick={() => (state === 'open' ? close() : readOnly ? undefined : setState('open'))}
      >
        <TokenIcon token={icon || 'agent'} />
      </button>
      {state !== 'closed' ? (
        <div
          ref={popRef}
          className="dir-ui-af-avatar-pop iii-ui-motion-dropdown"
          data-state={state === 'closing' ? 'closed' : 'open'}
          role="dialog"
          aria-label="Agent avatars"
          inert={state === 'closing'}
          onAnimationEnd={() => {
            if (state === 'closing') setState('closed')
          }}
        >
          <p className="dir-ui-af-avatar-pop-title">Choose an avatar</p>
          <div className="dir-ui-af-avatar-grid">
            {LOGO_PRESETS.map((preset) => (
              <button
                key={preset.token}
                type="button"
                aria-pressed={icon === preset.token}
                aria-label={preset.token}
                className="dir-ui-af-avatar-option"
                data-color={selectedColor}
                onClick={() => onIconChange(preset)}
              >
                <TokenIcon token={preset.token} />
              </button>
            ))}
            <button
              type="button"
              aria-pressed={icon === ''}
              aria-label="No avatar"
              className="dir-ui-af-avatar-option"
              data-color={selectedColor}
              onClick={() => onIconChange(null)}
            >
              <X aria-hidden />
            </button>
          </div>
          <div className="dir-ui-af-avatar-colors" role="radiogroup" aria-label="Avatar color">
            {AGENT_COLORS.map((option) => (
              <label key={option.id} className="dir-ui-af-avatar-color" data-color={option.id}>
                <input
                  type="radio"
                  name="agent-avatar-color"
                  value={option.id}
                  aria-label={option.label}
                  checked={selectedColor === option.id}
                  disabled={readOnly}
                  onChange={() => onColorChange(option.id)}
                />
              </label>
            ))}
          </div>
        </div>
      ) : null}
    </div>
  )
}

function useAutoResizeTextarea(value: string) {
  const ref = useRef<HTMLTextAreaElement>(null)
  useLayoutEffect(() => {
    const textarea = ref.current
    if (!textarea) return
    textarea.style.height = '0px'
    textarea.style.height = `${textarea.scrollHeight}px`
  }, [value])
  // …and when the pane width changes the wrap.
  useLayoutEffect(() => {
    const textarea = ref.current
    if (!textarea || typeof ResizeObserver === 'undefined') return
    let width = textarea.clientWidth
    const observer = new ResizeObserver(() => {
      if (textarea.clientWidth === width) return
      width = textarea.clientWidth
      textarea.style.height = '0px'
      textarea.style.height = `${textarea.scrollHeight}px`
    })
    observer.observe(textarea)
    return () => observer.disconnect()
  }, [])
  return ref
}

/** The agent's name and description are edited where they are read: plain
 * text you click into (no box, no pencil, no edit mode). The description
 * grows with its text. Escape puts back what was there when you came in. */
function InlineTextField({
  id,
  name,
  label,
  value,
  placeholder,
  readOnly,
  multiline = false,
  onChange,
  onBegin,
  onCancel,
}: {
  id: string
  name: string
  label: string
  value: string
  placeholder: string
  readOnly: boolean
  multiline?: boolean
  onChange: (next: string) => void
  /** Focus arrived: remember the document as it is now. */
  onBegin: () => void
  /** Escape: put that document back, byte for byte (rewriting just the field
      could re-quote the frontmatter and leave the draft "changed"). */
  onCancel: () => void
}) {
  const textareaRef = useAutoResizeTextarea(value)
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (event.key === 'Escape') {
      onCancel()
      event.currentTarget.blur()
      event.preventDefault()
      return
    }
    if ((!multiline && event.key === 'Enter') || (event.metaKey && event.key === 'Enter')) {
      event.preventDefault()
      event.currentTarget.blur()
    }
  }
  const common = {
    id,
    name,
    value,
    placeholder,
    readOnly,
    spellCheck: multiline,
    autoComplete: 'off',
    'aria-label': label,
    onFocus: onBegin,
    onKeyDown,
  }
  return multiline ? (
    <textarea
      {...common}
      ref={textareaRef}
      rows={1}
      className="dir-inplace dir-inplace-description"
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  ) : (
    <input
      {...common}
      required
      className="dir-inplace dir-inplace-name"
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  )
}

/** The system prompt is the profile's markdown body: edited in the same
 * Monaco editor as skills, filling its tab. */
function SystemPromptEditor({ draft, editDraft, readOnly }: Pick<FormContext, 'draft' | 'editDraft' | 'readOnly'>) {
  const rawBody = frontmatterBody(draft)
  const separator = rawBody.startsWith('\r\n') ? '\r\n' : rawBody.startsWith('\n') ? '\n' : ''
  const value = rawBody.slice(separator.length)
  return (
    <CodeEditor
      value={value}
      onChange={(next) => editDraft(setFrontmatterBody(draft, `${separator}${next}`))}
      language="markdown"
      fill
      fontSize={14}
      readOnly={readOnly}
      aria-label="Instructions"
      placeholder="Describe the agent’s role, constraints and working style…"
      className="dir-agent-prompt-editor"
    />
  )
}

type PickerList = 'selected' | 'available'

type PickerRow = PickItem & { missing?: boolean }

/** Copy for one picker instance (skills or preloaded functions). */
interface PickerCopy {
  /** Form field name for the native checkboxes (`skills`, `functions`). */
  field: string
  /** Plural noun in messages ("skills", "functions"). */
  plural: string
  placeholder: string
  loadError: string
  loading: string
  missing: string
  emptySelected: (needle: boolean) => string
  emptyAvailable: (needle: boolean) => string
  /** Render labels in the mono voice (function ids are technical data). */
  mono?: boolean
}

const SKILLS_COPY: PickerCopy = {
  field: 'skills',
  plural: 'skills',
  placeholder: 'Search skills',
  loadError: 'Unable to load the skill list. Your selection is kept and saves as it is.',
  loading: 'Loading skills…',
  missing: 'Not in the skill list any more.',
  emptySelected: (needle) =>
    needle ? 'No selected skills match.' : 'None yet. Chats with this agent fetch skills when a task needs them.',
  emptyAvailable: (needle) => (needle ? 'No other skills match.' : 'Every skill is selected.'),
}

const FUNCTIONS_COPY: PickerCopy = {
  field: 'functions',
  plural: 'functions',
  placeholder: 'Search functions, like coder::',
  loadError: 'Unable to read the function registry. Your selection is kept and saves as it is.',
  loading: 'Loading functions…',
  missing: 'Not registered right now. Chats are told it is unavailable.',
  emptySelected: (needle) =>
    needle ? 'No selected functions match.' : 'None yet. Chats with this agent find functions through search.',
  emptyAvailable: (needle) => (needle ? 'No other functions match.' : 'Every registered function is selected.'),
  mono: true,
}

function isPrintableKey(event: KeyboardEvent<HTMLElement>) {
  return event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey
}

/** One checkbox row: the library's anatomy (44px, title + one-line
 * description), with the check leading. */
function PickerRowView({
  item,
  field,
  checked,
  disabled,
  tabStop,
  mono,
  onChange,
  onFocus,
  register,
}: {
  item: PickerRow
  field: string
  checked: boolean
  disabled: boolean
  /** Roving tabindex: exactly one row per group sits in the Tab order. */
  tabStop: boolean
  mono?: boolean
  onChange: () => void
  onFocus: () => void
  register: (id: string, element: HTMLInputElement | null) => void
}) {
  return (
    <label className="dir-picker-row" data-mono={mono ? '' : undefined}>
      <input
        ref={(element) => register(item.id, element)}
        type="checkbox"
        name={field}
        value={item.id}
        checked={checked}
        disabled={disabled}
        tabIndex={tabStop ? 0 : -1}
        data-picker-row={item.id}
        className="dir-picker-native"
        onChange={onChange}
        onFocus={onFocus}
      />
      <span className="dir-picker-check" aria-hidden="true" data-checked={checked ? '' : undefined}>
        <Check />
      </span>
      <span className="dir-picker-text">
        <span className="dir-picker-title">
          <span className="dir-picker-name">{item.label}</span>
          {item.missing ? <span className="dir-picker-missing">missing</span> : null}
        </span>
        {item.desc ? <span className="dir-picker-desc">{item.desc}</span> : null}
      </span>
    </label>
  )
}

function PickerGroup({
  title,
  items,
  field,
  checked,
  disabled,
  empty,
  tabStopId,
  mono,
  onToggle,
  onRowFocus,
  register,
}: {
  title: string
  items: PickerRow[]
  field: string
  checked: boolean
  disabled: boolean
  empty: string
  tabStopId: string | null
  mono?: boolean
  onToggle: (id: string) => void
  onRowFocus: (id: string) => void
  register: (id: string, element: HTMLInputElement | null) => void
}) {
  const headId = useId()
  return (
    <section className="dir-picker-group" aria-labelledby={headId}>
      <h3 className="dir-picker-head" id={headId}>
        {title} <span className="dir-picker-count">{items.length}</span>
      </h3>
      {items.length === 0 ? (
        <p className="dir-picker-empty">{empty}</p>
      ) : (
        // biome-ignore lint/a11y/noRedundantRoles: keep list semantics where host styles drop markers
        <ul className="dir-picker-rows" role="list">
          {items.map((item) => (
            <li key={item.id}>
              <PickerRowView
                item={item}
                field={field}
                checked={checked}
                disabled={disabled}
                tabStop={item.id === tabStopId}
                mono={mono}
                onChange={() => onToggle(item.id)}
                onFocus={() => onRowFocus(item.id)}
                register={register}
              />
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

/**
 * Search + one outlined list in two groups (Selected, then Available).
 * Checking a row moves it between the groups at once — no animation, the
 * focus stays on the slot it left. Fully keyboard operable:
 *
 *   search: ↓/↑ enter the list · Enter adds the first available match
 *           (the query stays, so `coder::` + Enter×N adds one per press) ·
 *           Esc clears the query
 *   rows:   ↓/↑ move (↑ from the top returns to search) · Home/End ·
 *           Space or Enter toggles · Esc returns to search · typing (or
 *           Backspace) goes straight back to the search box
 *
 * Each group is one Tab stop (roving tabindex), so a long function registry
 * never becomes hundreds of Tab presses.
 */
function PickerEditor({
  copy,
  items,
  error,
  missing,
  readOnly,
  isSelected,
  onToggle,
}: {
  copy: PickerCopy
  items: PickItem[] | null
  error: boolean
  missing: string[]
  readOnly: boolean
  isSelected: (id: string) => boolean
  onToggle: (id: string) => void
}) {
  const [filter, setFilter] = useState('')
  const [active, setActive] = useState<Record<PickerList, string | null>>({ selected: null, available: null })
  const searchRef = useRef<HTMLInputElement | null>(null)
  const rowsRef = useRef(new Map<string, HTMLInputElement>())
  const pendingFocusRef = useRef<{ list: PickerList; index: number } | null>(null)
  const listsId = useId()
  const needle = filter.trim().toLowerCase()
  const matches = (item: PickItem) =>
    !needle ||
    item.id.toLowerCase().includes(needle) ||
    item.label.toLowerCase().includes(needle) ||
    (item.desc ?? '').toLowerCase().includes(needle)

  const available: PickerRow[] = (items ?? []).filter((item) => !isSelected(item.id) && matches(item))
  const selectedItems: PickerRow[] = [
    ...(items ?? []).filter((item) => isSelected(item.id) && matches(item)),
    ...missing.map((id) => ({ id, label: id, desc: copy.missing, missing: true })).filter(matches),
  ]
  const rows: Record<PickerList, PickerRow[]> = { selected: selectedItems, available }
  const order = [...selectedItems, ...available].map((item) => item.id)

  const register = useCallback((id: string, element: HTMLInputElement | null) => {
    if (element) rowsRef.current.set(id, element)
    else rowsRef.current.delete(id)
  }, [])

  const focusRow = (id: string | undefined) => {
    if (!id) return false
    const element = rowsRef.current.get(id)
    if (!element) return false
    element.focus()
    element.closest('label')?.scrollIntoView?.({ block: 'nearest' })
    return true
  }
  const focusSearch = () => searchRef.current?.focus()

  // After a keyboard toggle the row re-renders in the other group; keep
  // the focus on the slot it left (or the search box when the group ran dry).
  useLayoutEffect(() => {
    const pending = pendingFocusRef.current
    if (!pending) return
    pendingFocusRef.current = null
    const list = rows[pending.list]
    const next = list[Math.min(pending.index, list.length - 1)]
    if (!next || !focusRow(next.id)) focusSearch()
  })

  /** Which group a row sits in right now, and at which slot. */
  const slotOf = (id: string): { list: PickerList; index: number } => {
    const list: PickerList = isSelected(id) || missing.includes(id) ? 'selected' : 'available'
    return { list, index: rows[list].findIndex((item) => item.id === id) }
  }
  const toggleFromKeyboard = (id: string) => {
    if (readOnly) return
    pendingFocusRef.current = slotOf(id)
    onToggle(id)
  }

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        if (focusRow(order[0])) event.preventDefault()
        return
      case 'ArrowUp':
        if (focusRow(order[order.length - 1])) event.preventDefault()
        return
      case 'Enter': {
        event.preventDefault()
        const first = available[0]
        if (!first || readOnly) return
        pendingFocusRef.current = null
        onToggle(first.id)
        return
      }
      case 'Escape':
        if (filter) {
          event.preventDefault()
          event.stopPropagation()
          setFilter('')
        }
        return
      default:
        return
    }
  }

  const onListsKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    const id = target.dataset?.pickerRow
    if (!id) return
    const index = order.indexOf(id)
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        focusRow(order[index + 1])
        return
      case 'ArrowUp':
        event.preventDefault()
        if (index <= 0) focusSearch()
        else focusRow(order[index - 1])
        return
      case 'Home':
        event.preventDefault()
        focusRow(order[0])
        return
      case 'End':
        event.preventDefault()
        focusRow(order[order.length - 1])
        return
      case 'Enter':
        event.preventDefault()
        toggleFromKeyboard(id)
        return
      case ' ':
        // Space is the checkbox's own toggle (fires on keyup); only arm the
        // focus follow-up so the row's slot keeps the focus afterwards.
        if (!readOnly) pendingFocusRef.current = slotOf(id)
        return
      case 'Escape':
        event.preventDefault()
        event.stopPropagation()
        focusSearch()
        return
      case 'Backspace':
        event.preventDefault()
        setFilter((current) => current.slice(0, -1))
        focusSearch()
        return
      default:
        if (isPrintableKey(event)) {
          event.preventDefault()
          setFilter((current) => current + event.key)
          focusSearch()
        }
    }
  }

  const tabStopFor = (list: PickerList) => {
    const current = active[list]
    if (current && rows[list].some((item) => item.id === current)) return current
    return rows[list][0]?.id ?? null
  }
  const onRowFocus = (list: PickerList) => (id: string) =>
    setActive((current) => (current[list] === id ? current : { ...current, [list]: id }))

  let body: ReactNode
  if (error) {
    body = (
      <p className="dir-picker-empty" role="status">
        {copy.loadError}
      </p>
    )
  } else if (items === null) {
    body = <p className="dir-picker-empty">{copy.loading}</p>
  } else {
    body = (
      <>
        <PickerGroup
          title="Selected"
          items={selectedItems}
          field={copy.field}
          checked
          disabled={readOnly}
          empty={copy.emptySelected(needle.length > 0)}
          tabStopId={tabStopFor('selected')}
          mono={copy.mono}
          onToggle={toggleFromKeyboard}
          onRowFocus={onRowFocus('selected')}
          register={register}
        />
        <PickerGroup
          title="Available"
          items={available}
          field={copy.field}
          checked={false}
          disabled={readOnly}
          empty={copy.emptyAvailable(needle.length > 0)}
          tabStopId={tabStopFor('available')}
          mono={copy.mono}
          onToggle={toggleFromKeyboard}
          onRowFocus={onRowFocus('available')}
          register={register}
        />
      </>
    )
  }

  return (
    <div className="dir-picker" data-picker={copy.field}>
      <label className="dir-lib-search dir-picker-search">
        <Search aria-hidden className="dir-lib-search-icon" />
        <input
          ref={searchRef}
          type="search"
          name={`${copy.field}_filter`}
          value={filter}
          onChange={(event) => setFilter(event.currentTarget.value)}
          onKeyDown={onSearchKeyDown}
          placeholder={copy.placeholder}
          aria-label={`Search ${copy.plural}`}
          aria-controls={listsId}
          aria-keyshortcuts="ArrowDown ArrowUp Enter Escape"
          spellCheck={false}
          autoComplete="off"
        />
        {filter ? (
          <button
            type="button"
            className="dir-lib-search-clear"
            aria-label="Clear search"
            onClick={() => setFilter('')}
          >
            <X aria-hidden />
          </button>
        ) : null}
      </label>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: arrow-key relay between the checkboxes inside */}
      <div id={listsId} className="dir-picker-list" onKeyDown={onListsKeyDown}>
        {body}
      </div>
    </div>
  )
}

/** One placeholder block; sizes mirror the control it stands in for. */
function Bar({ w, h = 10 }: { w: string | number; h?: number }) {
  return <Skeleton style={{ display: 'block', width: w, height: h }} />
}

function AgentFormSkeletonLayout() {
  return (
    <div className="dir-agent dir-agent-skeleton">
      <div className="dir-agent-head">
        <Bar w={48} h={48} />
        <div className="dir-agent-identity">
          <Bar w="min(52%, 280px)" h={22} />
          <Bar w="min(72%, 420px)" h={12} />
        </div>
      </div>
      <div className="dir-agent-tabs-skeleton">
        {[72, 104, 56, 80].map((w) => (
          <Bar key={w} w={w} h={12} />
        ))}
      </div>
      <div className="dir-agent-settings">
        {[0, 1, 2, 3].map((row) => (
          <div key={row} className="dir-agent-setting">
            <div className="dir-agent-setting-copy">
              <Bar w={96} h={12} />
              <Bar w="min(80%, 220px)" h={10} />
            </div>
            <Bar w="100%" h={34} />
          </div>
        ))}
      </div>
    </div>
  )
}

export function AgentFormSkeleton() {
  return (
    <div className="dir-ui-af-loading t-skel" data-state="loading" role="status" aria-label="Loading agent">
      <div className="t-skel-skeleton" aria-hidden="true">
        <AgentFormSkeletonLayout />
      </div>
      <div className="t-skel-content" aria-hidden="true" />
    </div>
  )
}

/** Server-side cap on `composer_placeholder` (iii-directory
    `AGENT_COMPOSER_PLACEHOLDER_MAX_CHARS`); the field enforces it too. */
export const COMPOSER_PLACEHOLDER_MAX_CHARS = 200

/**
 * Write the profile's composer example as typed, or drop the key when it is
 * blank so the chat falls back to its generic hint. Every other frontmatter
 * key and the body are left untouched.
 */
export function withComposerPlaceholder(draft: string, next: string): string {
  return next.trim()
    ? setFrontmatterField(draft, 'composer_placeholder', next)
    : withoutFrontmatterFields(draft, ['composer_placeholder'])
}

function ComposerExampleField({
  id,
  value,
  readOnly,
  onChange,
}: {
  id: string
  value: string
  readOnly: boolean
  onChange: (next: string) => void
}) {
  const textareaRef = useAutoResizeTextarea(value)
  return (
    <textarea
      ref={textareaRef}
      id={id}
      name="composer_placeholder"
      value={value}
      rows={1}
      maxLength={COMPOSER_PLACEHOLDER_MAX_CHARS}
      readOnly={readOnly}
      aria-describedby={`${id}-hint`}
      placeholder="Example: Review my workers and suggest useful tests."
      className="dir-ui-af-text-input"
      onChange={(event) => onChange(event.currentTarget.value)}
    />
  )
}

type AgentTab = 'overview' | 'prompt' | 'skills' | 'functions'

export function AgentForm(ctx: FormContext) {
  const {
    host,
    draft,
    editDraft,
    readOnly,
    fieldId,
    nameValue,
    descriptionValue,
    setName,
    setDescription,
    creating,
    saving,
    deleting,
    onRemove,
    onDuplicate,
    entryKey,
  } = ctx
  const skills = readFrontmatterStringList(draft, 'skills').values
  const functions = readFrontmatterStringList(draft, 'functions').values
  const extendsId = readFrontmatterField(draft, ['extends']).value.trim()
  const model = readFrontmatterField(draft, ['model']).value.trim()
  const reasoningEffort = readFrontmatterField(draft, ['reasoning_effort']).value.trim() || 'default'
  const icon = readFrontmatterField(draft, ['icon']).value.trim()
  const color = readFrontmatterField(draft, ['color']).value.trim()
  const hidden = readFrontmatterField(draft, ['hidden']).value.trim() === 'true'
  const composerPlaceholder = readFrontmatterField(draft, ['composer_placeholder']).value
  const skillCatalog = useCatalog(host, fetchSkills)
  const functionCatalog = useCatalog(host, fetchFunctions)
  const modelCatalog = useCatalog(host, fetchModels)
  const agentCatalog = useCatalog(host, fetchAgents)
  const derived = useMemo(() => slugify(nameValue), [nameValue])
  const [tab, setTab] = useState<AgentTab>('overview')
  const draftRef = useRef(draft)
  draftRef.current = draft
  const commitDraft = (next: string) => {
    draftRef.current = next
    editDraft(next)
  }

  // Escape in the name/description puts back the whole document as it was
  // when the field took focus.
  const inPlaceSnapshot = useRef<string | null>(null)
  const beginInPlace = () => {
    inPlaceSnapshot.current = draftRef.current
  }
  const cancelInPlace = () => {
    const snapshot = inPlaceSnapshot.current
    if (snapshot !== null && snapshot !== draftRef.current) commitDraft(snapshot)
  }

  const setModel = (next: string) => {
    const current = draftRef.current
    commitDraft(
      next
        ? setFrontmatterField(current, 'model', next)
        : withoutFrontmatterFields(current, ['model', 'reasoning_effort']),
    )
  }
  const setReasoningEffort = (next: string) => {
    const current = draftRef.current
    commitDraft(
      next && next !== 'default'
        ? setFrontmatterField(current, 'reasoning_effort', next)
        : withoutFrontmatterFields(current, ['reasoning_effort']),
    )
  }
  const setExtends = (next: string) => {
    const current = draftRef.current
    commitDraft(
      next ? setFrontmatterField(current, 'extends', next, true) : withoutFrontmatterFields(current, ['extends']),
    )
  }
  const setAvatar = (preset: { emoji: string; token: string } | null) => {
    if (preset === null) {
      editDraft(withoutFrontmatterFields(draft, ['logo', 'icon']))
      return
    }
    const withLogo = setFrontmatterField(draft, 'logo', preset.emoji)
    editDraft(setFrontmatterField(withLogo, 'icon', preset.token, true))
  }
  const setHidden = (next: boolean) => {
    editDraft(next ? setFrontmatterField(draft, 'hidden', 'true', true) : withoutFrontmatterFields(draft, ['hidden']))
  }
  const setComposerPlaceholder = (next: string) => {
    commitDraft(withComposerPlaceholder(draftRef.current, next))
  }
  const setAvatarColor = (next: AgentColor) => {
    editDraft(setFrontmatterField(draft, 'color', next, true))
  }
  const equivalentSkillIds = (catalogId: string) => {
    const ids = [catalogId]
    if (catalogId.endsWith('/index')) {
      ids.push(catalogId.slice(0, -'/index'.length))
    } else {
      ids.push(`${catalogId}/index`)
    }
    return ids
  }
  const isSkillSelected = (id: string) => equivalentSkillIds(id).some((candidate) => skills.includes(candidate))
  const toggleSkill = (id: string) => {
    const equivalents = new Set(equivalentSkillIds(id))
    const next = isSkillSelected(id) ? skills.filter((skill) => !equivalents.has(skill)) : [...skills, id]
    editDraft(setFrontmatterStringList(draft, 'skills', next))
  }
  const knownSkillIds = new Set((skillCatalog.items ?? []).flatMap((item) => equivalentSkillIds(item.id)))
  const missingSkills = skillCatalog.items === null ? [] : skills.filter((id) => !knownSkillIds.has(id))
  // Preloaded functions: verbatim engine function ids, no aliasing.
  const isFunctionSelected = (id: string) => functions.includes(id)
  const toggleFunction = (id: string) => {
    const current = draftRef.current
    const currentFunctions = readFrontmatterStringList(current, 'functions').values
    const next = currentFunctions.includes(id) ? currentFunctions.filter((fn) => fn !== id) : [...currentFunctions, id]
    commitDraft(setFrontmatterStringList(current, 'functions', next))
  }
  const knownFunctionIds = new Set((functionCatalog.items ?? []).map((item) => item.id))
  const missingFunctions = functionCatalog.items === null ? [] : functions.filter((id) => !knownFunctionIds.has(id))
  const pickerOptions = useMemo(() => {
    const options = modelCatalog.items ?? []
    if (!model || options.some((option) => option.id === model)) return options
    return [{ id: model, label: model }, ...options]
  }, [model, modelCatalog.items])
  const modelKnown = !model || modelCatalog.items === null || modelCatalog.items.some((option) => option.id === model)
  // A profile never extends itself; an unknown current value stays visible
  // (same trick as `pickerOptions`) so the author can see what to fix.
  const parentOptions = useMemo(() => {
    const rows = (agentCatalog.items ?? []).filter((row) => row.id !== entryKey)
    if (!extendsId || rows.some((row) => row.id === extendsId)) return rows
    return [{ id: extendsId, name: extendsId }, ...rows]
  }, [agentCatalog.items, entryKey, extendsId])
  // Server-side verdict on the SAVED chain; refreshes with the catalog on
  // the next form mount.
  const inheritanceError = agentCatalog.items?.find((row) => row.id === entryKey)?.inheritance_error ?? null
  const catalogsLoading =
    (skillCatalog.items === null && !skillCatalog.error) ||
    (functionCatalog.items === null && !functionCatalog.error) ||
    (modelCatalog.items === null && !modelCatalog.error)

  const modelLabel = model
    ? (pickerOptions.find((option) => option.id === model)?.label ?? model)
    : 'the session’s model'
  const summary = [
    modelLabel,
    countLabel(skills.length, { one: 'skill', other: 'skills' }),
    countLabel(functions.length, { one: 'function', other: 'functions' }),
    ...(extendsId ? [`based on ${extendsId}`] : []),
  ].join(' · ')

  const settings = (
    <>
      <div className="dir-agent-settings">
        <div className="dir-agent-setting">
          <div className="dir-agent-setting-copy">
            <span className="dir-agent-setting-label">Model</span>
            <span className="dir-agent-setting-hint">
              {modelKnown
                ? 'The model new chats start on. Leave empty to use the session’s.'
                : 'This model is not in the model list any more.'}
            </span>
          </div>
          <div className="dir-agent-setting-control">
            <ModelPicker
              value={model || null}
              options={pickerOptions}
              thinkingLevel={reasoningEffort}
              onChange={setModel}
              onThinkingLevelChange={setReasoningEffort}
              disabled={readOnly || modelCatalog.error}
              loading={modelCatalog.items === null && !modelCatalog.error}
              showRefresh={false}
              showProviderConfiguration={false}
              showReasoningEffort
              placeholder="Session default"
              className="dir-agent-picker"
            />
            {model && !readOnly ? (
              <IconButton label="Use the session default model" onClick={() => setModel('')}>
                <X />
              </IconButton>
            ) : null}
          </div>
        </div>

        <div className="dir-agent-setting">
          <div className="dir-agent-setting-copy">
            <span className="dir-agent-setting-label">Based on</span>
            <span className="dir-agent-setting-hint">
              {inheritanceError ? 'The saved chain does not resolve.' : 'Start from another agent and add to it.'}
            </span>
          </div>
          <div className="dir-agent-setting-control">
            <Select
              className="dir-agent-picker"
              aria-label="Based on"
              aria-busy={agentCatalog.items === null && !agentCatalog.error}
              value={extendsId || undefined}
              options={parentOptions.map((row) => ({
                value: row.id,
                label: row.name && row.name !== row.id ? `${row.name} (${row.id})` : row.id,
              }))}
              placeholder="None"
              allowEmpty
              emptyLabel="None"
              onClear={() => setExtends('')}
              disabled={readOnly || agentCatalog.error}
              onChange={setExtends}
            />
            {inheritanceError ? <p className="dir-agent-setting-error">{inheritanceError}</p> : null}
          </div>
        </div>

        <div className="dir-agent-setting">
          <div className="dir-agent-setting-copy">
            <label className="dir-agent-setting-label" htmlFor={`${fieldId}-composer-placeholder`}>
              Composer example
            </label>
            <span className="dir-agent-setting-hint" id={`${fieldId}-composer-placeholder-hint`}>
              Shown in an empty chat box for this agent. Never sent, and not inherited.
            </span>
          </div>
          <div className="dir-agent-setting-control">
            <ComposerExampleField
              id={`${fieldId}-composer-placeholder`}
              value={composerPlaceholder}
              readOnly={readOnly}
              onChange={setComposerPlaceholder}
            />
          </div>
        </div>

        <div className="dir-agent-setting">
          <div className="dir-agent-setting-copy">
            <label className="dir-agent-setting-label" htmlFor={`${fieldId}-hidden`}>
              Hide from the new-chat list
            </label>
            <span className="dir-agent-setting-hint" id={`${fieldId}-hidden-hint`}>
              Still usable as a base for other agents.
            </span>
          </div>
          <div className="dir-agent-setting-control is-switch">
            <Switch
              id={`${fieldId}-hidden`}
              aria-describedby={`${fieldId}-hidden-hint`}
              checked={hidden}
              disabled={readOnly}
              onChange={(event) => setHidden(event.currentTarget.checked)}
            />
          </div>
        </div>
      </div>
      <p className="dir-agent-summary">
        <span className="dir-agent-summary-label">New chats start with</span> {summary}
      </p>
      {onRemove ? (
        <div className="dir-agent-danger">
          <ShadcnButton
            variant="ghost"
            size="sm"
            className="dir-delete"
            disabled={saving || deleting}
            onClick={onRemove}
          >
            <Trash2 aria-hidden />
            Delete agent
          </ShadcnButton>
        </div>
      ) : null}
    </>
  )

  const [copied, setCopied] = useState(false)
  const copyTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(copyTimer.current), [])
  const copyId = () => {
    if (!entryKey) return
    void copyText(entryKey).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.clearTimeout(copyTimer.current)
      copyTimer.current = window.setTimeout(() => setCopied(false), 1400)
    })
  }

  return (
    <div
      className={`dir-ui-af-loading t-skel${catalogsLoading ? '' : ' is-revealed'}`}
      data-state={catalogsLoading ? 'loading' : 'loaded'}
      aria-busy={catalogsLoading}
    >
      <div className="t-skel-skeleton" aria-hidden={!catalogsLoading}>
        <AgentFormSkeletonLayout />
      </div>
      <div className="t-skel-content" aria-hidden={catalogsLoading} inert={catalogsLoading}>
        <div className="dir-agent">
          <header className="dir-agent-head">
            <AvatarPicker
              icon={icon}
              color={color}
              readOnly={readOnly}
              onIconChange={setAvatar}
              onColorChange={setAvatarColor}
            />
            <div className="dir-agent-identity">
              <InlineTextField
                id={`${fieldId}-name`}
                name="name"
                label="Agent name"
                onBegin={beginInPlace}
                onCancel={cancelInPlace}
                value={nameValue}
                placeholder="Untitled agent"
                readOnly={readOnly}
                onChange={setName}
              />
              <InlineTextField
                id={`${fieldId}-description`}
                name="description"
                label="Agent description"
                onBegin={beginInPlace}
                onCancel={cancelInPlace}
                value={descriptionValue}
                placeholder="Add a description"
                readOnly={readOnly}
                multiline
                onChange={setDescription}
              />
              <p className="dir-agent-file">
                {creating
                  ? derived
                    ? `Saved as agents/${derived}.md`
                    : 'The file name comes from the name.'
                  : `agents/${entryKey}.md`}
              </p>
            </div>
            {!creating ? (
              <ShadcnDropdownMenu>
                <ShadcnDropdownMenuTrigger
                  render={
                    <ShadcnButton variant="ghost" size="icon-sm" aria-label="More agent actions">
                      <Ellipsis aria-hidden />
                    </ShadcnButton>
                  }
                />
                <ShadcnDropdownMenuContent align="end" sideOffset={6} className="dir-menu">
                  <ShadcnDropdownMenuGroup>
                    <ShadcnDropdownMenuItem closeOnClick={false} onClick={copyId}>
                      {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
                      {copied ? 'Copied' : 'Copy id'}
                    </ShadcnDropdownMenuItem>
                    {onDuplicate ? (
                      <ShadcnDropdownMenuItem onClick={onDuplicate}>
                        <CopyPlus aria-hidden />
                        Duplicate
                      </ShadcnDropdownMenuItem>
                    ) : null}
                  </ShadcnDropdownMenuGroup>
                </ShadcnDropdownMenuContent>
              </ShadcnDropdownMenu>
            ) : null}
          </header>

          <ShadcnTabs value={tab} onValueChange={(next) => setTab(next as AgentTab)} className="dir-agent-tabs">
            <ShadcnTabsList variant="line" activateOnFocus className="dir-agent-tablist">
              <ShadcnTabsTrigger value="overview">Overview</ShadcnTabsTrigger>
              <ShadcnTabsTrigger value="prompt">Instructions</ShadcnTabsTrigger>
              <ShadcnTabsTrigger value="skills">
                Skills <span className="dir-agent-tab-count">{skills.length}</span>
              </ShadcnTabsTrigger>
              <ShadcnTabsTrigger value="functions">
                Functions <span className="dir-agent-tab-count">{functions.length}</span>
              </ShadcnTabsTrigger>
            </ShadcnTabsList>
            <ShadcnTabsContent value="overview" keepMounted className="dir-agent-panel is-scroll">
              {settings}
            </ShadcnTabsContent>
            <ShadcnTabsContent value="prompt" keepMounted className="dir-agent-panel is-editor">
              <p className="dir-agent-panel-hint">
                What every chat with this agent follows. With a base agent, empty instructions run the base’s alone.
              </p>
              <SystemPromptEditor draft={draft} editDraft={editDraft} readOnly={readOnly} />
            </ShadcnTabsContent>
            <ShadcnTabsContent value="skills" keepMounted className="dir-agent-panel is-fill">
              <p className="dir-agent-panel-hint">Selected skills load into every chat this agent starts.</p>
              <PickerEditor
                copy={SKILLS_COPY}
                items={skillCatalog.items}
                error={skillCatalog.error}
                missing={missingSkills}
                readOnly={readOnly}
                isSelected={isSkillSelected}
                onToggle={toggleSkill}
              />
            </ShadcnTabsContent>
            <ShadcnTabsContent value="functions" keepMounted className="dir-agent-panel is-fill">
              <p className="dir-agent-panel-hint">
                Functions this agent calls all the time. Their contracts load up front, so it calls them without
                searching first.
              </p>
              <PickerEditor
                copy={FUNCTIONS_COPY}
                items={functionCatalog.items}
                error={functionCatalog.error}
                missing={missingFunctions}
                readOnly={readOnly}
                isSelected={isFunctionSelected}
                onToggle={toggleFunction}
              />
            </ShadcnTabsContent>
          </ShadcnTabs>
        </div>
      </div>
    </div>
  )
}
