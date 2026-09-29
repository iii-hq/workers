import {
  Button,
  CollapsibleCard,
  CollapsibleCardContent,
  CollapsibleCardTrigger,
  type Host,
  IconButton,
  Kbd,
  type ModelOption,
  ModelPicker,
  SearchField,
  Select,
  Skeleton,
  Switch,
} from '@iii-dev/console-ui'
import {
  Bot,
  Check,
  ChevronDown,
  ClipboardCheck,
  CodeXml,
  Database,
  FileText,
  FlaskConical,
  type LucideIcon,
  Palette,
  Pencil,
  Search,
  Terminal,
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

function useCatalog<T>(host: Host, fetch: (host: Host) => Promise<T[]>): CatalogState<T> {
  const [items, setItems] = useState<T[] | null>(null)
  const [error, setError] = useState(false)

  useEffect(() => {
    let cancelled = false
    setError(false)
    fetch(host)
      .then((next) => {
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
    .trigger<{
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
  host.iii.trigger<{ functions?: EngineFunctionRow[] }>('engine::functions::list', {}).then((out) =>
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
  host.iii.trigger<{ agents?: AgentCatalogRow[] }>('directory::agents::list').then((out) => out.agents ?? [])

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
  host.iii.trigger<{ models?: CatalogModelRow[] }>('router::models::list', {}).then((out) =>
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
        aria-label="Choose agent profile avatar"
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
          aria-label="Agent profile avatars"
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
                title={preset.token}
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
              title="No avatar"
              className="dir-ui-af-avatar-option"
              data-color={selectedColor}
              onClick={() => onIconChange(null)}
            >
              <X aria-hidden />
            </button>
          </div>
          <div className="dir-ui-af-avatar-colors" role="radiogroup" aria-label="Avatar color">
            {AGENT_COLORS.map((option) => (
              <label key={option.id} className="dir-ui-af-avatar-color" data-color={option.id} title={option.label}>
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
  // biome-ignore lint/correctness/useExhaustiveDependencies: resize after controlled value changes
  useLayoutEffect(() => {
    const textarea = ref.current
    if (!textarea) return
    textarea.style.height = '0px'
    textarea.style.height = `${textarea.scrollHeight}px`
  }, [value])
  return ref
}

function InlineTextField({
  id,
  name,
  value,
  placeholder,
  readOnly,
  multiline = false,
  onChange,
}: {
  id: string
  name: string
  value: string
  placeholder: string
  readOnly: boolean
  multiline?: boolean
  onChange: (next: string) => void
}) {
  const [editing, setEditing] = useState(false)
  const initialRef = useRef(value)
  const inputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useAutoResizeTextarea(value)

  useEffect(() => {
    if (!editing) return
    const field = multiline ? textareaRef.current : inputRef.current
    field?.focus()
    if (!multiline && field instanceof HTMLInputElement) field.select()
  }, [editing, multiline, textareaRef])

  const begin = () => {
    if (readOnly) return
    initialRef.current = value
    setEditing(true)
  }
  const cancel = () => {
    onChange(initialRef.current)
    setEditing(false)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      cancel()
      return
    }
    if ((!multiline && event.key === 'Enter') || (event.metaKey && event.key === 'Enter')) {
      event.preventDefault()
      setEditing(false)
    }
  }

  if (editing) {
    return multiline ? (
      <textarea
        ref={textareaRef}
        id={id}
        name={name}
        value={value}
        rows={1}
        aria-label="Agent profile description"
        placeholder={placeholder}
        className="dir-ui-af-inline-input dir-ui-af-inline-description-input"
        onChange={(event) => onChange(event.currentTarget.value)}
        onBlur={() => setEditing(false)}
        onKeyDown={onKeyDown}
      />
    ) : (
      <input
        ref={inputRef}
        id={id}
        name={name}
        value={value}
        required
        spellCheck={false}
        aria-label="Agent profile name"
        placeholder={placeholder}
        className="dir-ui-af-inline-input dir-ui-af-inline-name-input"
        onChange={(event) => onChange(event.currentTarget.value)}
        onBlur={() => setEditing(false)}
        onKeyDown={onKeyDown}
      />
    )
  }

  const display = value.trim() || placeholder
  if (readOnly) {
    return (
      <div className={`dir-ui-af-inline-display${multiline ? ' description' : ' name'}`}>
        <span className={value.trim() ? '' : 'placeholder'}>{display}</span>
      </div>
    )
  }

  return (
    <button
      type="button"
      className={`dir-ui-af-inline-display${multiline ? ' description' : ' name'}`}
      aria-label={`Edit agent profile ${multiline ? 'description' : 'name'}`}
      onClick={begin}
    >
      <span className={value.trim() ? '' : 'placeholder'}>{display}</span>
      <Pencil aria-hidden className="dir-ui-af-pencil" />
    </button>
  )
}

function CollapsibleSection({
  title,
  description,
  summary,
  defaultOpen = true,
  children,
}: {
  title: string
  description: string
  summary?: string
  defaultOpen?: boolean
  children: ReactNode
}) {
  return (
    <CollapsibleCard className="dir-ui-af-disclosure" defaultOpen={defaultOpen}>
      <CollapsibleCardTrigger className="dir-ui-af-disclosure-head">
        <span className="dir-ui-af-disclosure-copy">
          <span className="dir-ui-af-disclosure-title">{title}</span>
          <span className="dir-ui-af-disclosure-description">{description}</span>
        </span>
        {summary ? <span className="dir-ui-af-disclosure-summary">{summary}</span> : null}
        <ChevronDown aria-hidden className="dir-ui-af-disclosure-chevron" />
      </CollapsibleCardTrigger>
      <CollapsibleCardContent>
        <div className="dir-ui-af-disclosure-content">{children}</div>
      </CollapsibleCardContent>
    </CollapsibleCard>
  )
}

function SystemPromptEditor({ draft, editDraft, readOnly }: Pick<FormContext, 'draft' | 'editDraft' | 'readOnly'>) {
  const rawBody = frontmatterBody(draft)
  const separator = rawBody.startsWith('\r\n') ? '\r\n' : rawBody.startsWith('\n') ? '\n' : ''
  const value = rawBody.slice(separator.length)
  const textareaRef = useAutoResizeTextarea(value)

  return (
    <textarea
      ref={textareaRef}
      name="system_prompt"
      value={value}
      rows={6}
      readOnly={readOnly}
      spellCheck
      aria-label="System prompt"
      placeholder="Describe the profile's role, constraints, and working style…"
      className="dir-ui-af-prompt"
      onChange={(event) => editDraft(setFrontmatterBody(draft, `${separator}${event.currentTarget.value}`))}
    />
  )
}

type PickerList = 'selected' | 'available'

type PickerRow = PickItem & { missing?: boolean }

/** Copy for one picker instance (skills or preloaded functions). */
interface PickerCopy {
  /** Form field name for the hidden native checkboxes (`skills`, `functions`). */
  field: string
  /** Plural noun in messages ("skills", "functions"). */
  plural: string
  placeholder: string
  loadError: string
  loading: string
  missing: string
  emptySelected: (needle: boolean, selectedCount: number) => string
  emptyAvailable: (needle: boolean) => string
  /** Render labels in the mono voice (function ids are technical data). */
  mono?: boolean
}

const SKILLS_COPY: PickerCopy = {
  field: 'skills',
  plural: 'skills',
  placeholder: 'Filter skills…',
  loadError: 'The skill catalog could not be loaded. Existing selections will be preserved when you save.',
  loading: 'Loading skills…',
  missing: 'This skill is not in the current catalog.',
  emptySelected: (needle, count) =>
    needle
      ? 'No selected skills match.'
      : count === 0
        ? 'None — sessions using this profile fetch skills on demand.'
        : 'No skills selected.',
  emptyAvailable: (needle) => (needle ? 'No available skills match.' : 'All skills selected.'),
}

const FUNCTIONS_COPY: PickerCopy = {
  field: 'functions',
  plural: 'functions',
  placeholder: 'Filter functions (e.g. coder::)…',
  loadError: 'The function registry could not be read. Existing selections will be preserved when you save.',
  loading: 'Loading functions…',
  missing: 'This function is not registered right now; sessions will be told it is unavailable.',
  emptySelected: (needle, count) =>
    needle
      ? 'No selected functions match.'
      : count === 0
        ? 'None — sessions discover every function through search as usual.'
        : 'No functions selected.',
  emptyAvailable: (needle) => (needle ? 'No registered functions match.' : 'All registered functions selected.'),
  mono: true,
}

function isPrintableKey(event: KeyboardEvent<HTMLElement>) {
  return event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey
}

function PickerRowView({
  item,
  field,
  checked,
  disabled,
  tabStop,
  mono,
  missing = false,
  onChange,
  onFocus,
  register,
}: {
  item: PickItem
  field: string
  checked: boolean
  disabled: boolean
  /** Roving tabindex: exactly one row per list sits in the Tab order. */
  tabStop: boolean
  mono?: boolean
  missing?: boolean
  onChange: () => void
  onFocus: () => void
  register: (id: string, element: HTMLInputElement | null) => void
}) {
  return (
    <label className="dir-ui-af-skill-row" data-mono={mono ? 'true' : undefined}>
      <input
        ref={(element) => register(item.id, element)}
        type="checkbox"
        name={field}
        value={item.id}
        checked={checked}
        disabled={disabled}
        tabIndex={tabStop ? 0 : -1}
        data-picker-row={item.id}
        className="dir-ui-af-skill-native"
        onChange={onChange}
        onFocus={onFocus}
      />
      <span className="dir-ui-af-skill-check" aria-hidden="true" data-checked={checked}>
        <Check />
      </span>
      <span className="dir-ui-af-skill-copy">
        <span className="dir-ui-af-skill-name" title={item.label}>
          {item.label}
          {missing ? <span className="dir-ui-af-missing">Missing</span> : null}
        </span>
        <span className="dir-ui-af-skill-description" title={item.desc ?? 'No description.'}>
          {item.desc ?? 'No description.'}
        </span>
      </span>
    </label>
  )
}

function PickerListView({
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
  return (
    <div className="dir-ui-af-skill-list-wrap">
      <div className="dir-ui-af-skill-list-head">
        <span>{title}</span>
        <span className="dir-ui-af-skill-count">{items.length}</span>
      </div>
      {/* The explicit role keeps list semantics when host styles remove markers. */}
      {/* biome-ignore lint/a11y/noRedundantRoles: preserve list semantics across embedded hosts */}
      <ul className="dir-ui-af-skill-list" role="list" aria-label={title}>
        {items.length === 0 ? (
          <li className="dir-ui-af-skill-empty">{empty}</li>
        ) : (
          items.map((item) => (
            <li key={item.id}>
              <PickerRowView
                item={item}
                field={field}
                checked={checked}
                disabled={disabled}
                tabStop={item.id === tabStopId}
                mono={mono}
                missing={item.missing}
                onChange={() => onToggle(item.id)}
                onFocus={() => onRowFocus(item.id)}
                register={register}
              />
            </li>
          ))
        )}
      </ul>
    </div>
  )
}

/**
 * Search + two transfer lists (Selected / Available), fully keyboard
 * operable:
 *
 *   search: ↓/↑ enter the lists · Enter adds the first available match
 *           (the filter stays, so `coder::` + Enter×N adds one per press) ·
 *           Esc clears the filter
 *   rows:   ↓/↑ move (across both lists; ↑ from the top returns to search)
 *           · Home/End · Space or Enter toggles · Esc returns to search ·
 *           typing (or Backspace) goes straight back to the search box
 *
 * Each list is one Tab stop (roving tabindex), so a long function registry
 * never becomes hundreds of Tab presses. After a keyboard toggle the focus
 * stays on the same slot of the same list, so repeated toggles flow.
 */
function PickerEditor({
  copy,
  items,
  error,
  selected,
  missing,
  readOnly,
  isSelected,
  onToggle,
}: {
  copy: PickerCopy
  items: PickItem[] | null
  error: boolean
  selected: string[]
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
  // `SearchField` forwards no ARIA beyond its label: the list link and the
  // key hints ride on the element itself.
  const attachSearch = useCallback(
    (el: HTMLInputElement | null) => {
      searchRef.current = el
      el?.setAttribute('aria-controls', listsId)
      el?.setAttribute('aria-keyshortcuts', 'ArrowDown ArrowUp Enter Escape')
    },
    [listsId],
  )
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
    element.scrollIntoView?.({ block: 'nearest' })
    return true
  }
  const focusSearch = () => searchRef.current?.focus()

  // After a keyboard toggle the row re-renders under the other list; keep
  // the focus on the slot it left (or the search box when the list ran dry).
  useLayoutEffect(() => {
    const pending = pendingFocusRef.current
    if (!pending) return
    pendingFocusRef.current = null
    const list = rows[pending.list]
    const next = list[Math.min(pending.index, list.length - 1)]
    if (!next || !focusRow(next.id)) focusSearch()
  })

  /** Which list a row sits in right now, and at which slot. */
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

  if (error) {
    return (
      <div className="dir-ui-af-catalog-message" role="status">
        {copy.loadError}
      </div>
    )
  }
  if (items === null) {
    return <div className="dir-ui-af-catalog-message">{copy.loading}</div>
  }

  const tabStopFor = (list: PickerList) => {
    const current = active[list]
    if (current && rows[list].some((item) => item.id === current)) return current
    return rows[list][0]?.id ?? null
  }
  const onRowFocus = (list: PickerList) => (id: string) =>
    setActive((current) => (current[list] === id ? current : { ...current, [list]: id }))

  return (
    <div className="dir-ui-af-skills" data-picker={copy.field}>
      <SearchField
        ref={attachSearch}
        name={`${copy.field}_filter`}
        value={filter}
        onChange={setFilter}
        onKeyDown={onSearchKeyDown}
        placeholder={copy.placeholder}
        aria-label={`Filter ${copy.plural}`}
      />
      <p className="dir-ui-af-picker-hint" aria-hidden="true">
        <Kbd>↑</Kbd>
        <Kbd>↓</Kbd> browse · <Kbd>Enter</Kbd> adds the first match · <Kbd>Space</Kbd> toggles · <Kbd>Esc</Kbd> back to
        search
      </p>
      <div id={listsId} className="dir-ui-af-picker-lists" onKeyDown={onListsKeyDown}>
        <PickerListView
          title="Selected"
          items={selectedItems}
          field={copy.field}
          checked
          disabled={readOnly}
          empty={copy.emptySelected(needle.length > 0, selected.length)}
          tabStopId={tabStopFor('selected')}
          mono={copy.mono}
          onToggle={toggleFromKeyboard}
          onRowFocus={onRowFocus('selected')}
          register={register}
        />
        <div className="dir-ui-af-skill-transfer" aria-hidden="true">
          <span>↓</span>
          <span>↑</span>
        </div>
        <PickerListView
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
      </div>
    </div>
  )
}

/** One placeholder block; sizes mirror the control it stands in for. */
function Bar({ w, h = 10 }: { w: string | number; h?: number }) {
  return <Skeleton style={{ display: 'block', width: w, height: h }} />
}

function SkeletonSkillList() {
  return (
    <div className="dir-ui-af-skill-list-wrap">
      <div className="dir-ui-af-skill-list-head">
        <Bar w={72} />
        <Bar w={16} />
      </div>
      {/* biome-ignore lint/a11y/noRedundantRoles: preserve list semantics across embedded hosts */}
      <ul className="dir-ui-af-skill-list" role="list">
        {[0, 1].map((row) => (
          <li key={row}>
            <div className="dir-ui-af-skeleton-skill-row">
              <Bar w={20} h={20} />
              <span className="dir-ui-af-skill-copy">
                <Bar w="42%" />
                <Bar w="72%" h={9} />
              </span>
            </div>
          </li>
        ))}
      </ul>
    </div>
  )
}

function SkeletonDisclosure({ kind }: { kind: 'prompt' | 'skills' }) {
  return (
    <section className="dir-ui-af-disclosure">
      <div className="dir-ui-af-disclosure-head">
        <span className="dir-ui-af-disclosure-copy">
          <Bar w={96} h={16} />
          <Bar w="min(70%, 300px)" />
        </span>
        <span className="dir-ui-af-disclosure-summary">
          <Bar w={52} h={8} />
        </span>
        <Bar w={16} h={16} />
      </div>
      <div className="dir-ui-af-disclosure-content">
        {kind === 'prompt' ? (
          <div className="dir-ui-af-prompt dir-ui-af-skeleton-prompt">
            <Bar w="88%" />
            <Bar w="68%" />
            <Bar w="42%" />
          </div>
        ) : (
          <div className="dir-ui-af-skills">
            <Bar w="100%" h={36} />
            <SkeletonSkillList />
            <div className="dir-ui-af-skill-transfer" aria-hidden="true">
              <Bar w={36} h={6} />
            </div>
            <SkeletonSkillList />
          </div>
        )}
      </div>
    </section>
  )
}

function AgentFormSkeletonLayout() {
  return (
    <div className="dir-ui-af dir-ui-af-skeleton">
      <div className="dir-ui-af-profile">
        <Bar w={56} h={56} />
        <div className="dir-ui-af-profile-copy">
          <Bar w="min(52%, 280px)" h={24} />
          <Bar w="min(72%, 420px)" h={12} />
          <Bar w="min(48%, 280px)" h={12} />
        </div>
      </div>

      <div className="dir-ui-af-aligned">
        <div className="dir-ui-af-settings">
          {[0, 1].map((cell) => (
            <div key={cell} className="dir-ui-af-field">
              <div className="dir-ui-af-field-head">
                <Bar w={52} h={12} />
                <Bar w={112} h={8} />
              </div>
              <Bar w="100%" h={36} />
            </div>
          ))}
          <div className="dir-ui-af-field is-wide">
            <div className="dir-ui-af-field-head">
              <Bar w={112} h={12} />
              <Bar w="min(80%, 360px)" h={8} />
            </div>
            <Bar w="100%" h={32} />
          </div>
          <div className="dir-ui-af-field is-wide is-switch">
            <Bar w={36} h={20} />
            <div className="dir-ui-af-field-head">
              <Bar w={52} h={12} />
              <Bar w="min(70%, 320px)" h={8} />
            </div>
          </div>
        </div>
        <SkeletonDisclosure kind="prompt" />
        <SkeletonDisclosure kind="skills" />
        <SkeletonDisclosure kind="skills" />
      </div>
    </div>
  )
}

export function AgentFormSkeleton() {
  return (
    <div className="dir-ui-af-loading t-skel" data-state="loading" role="status" aria-label="Loading agent profile">
      <div className="t-skel-skeleton is-pulsing" aria-hidden="true">
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
  const draftRef = useRef(draft)
  draftRef.current = draft
  const commitDraft = (next: string) => {
    draftRef.current = next
    editDraft(next)
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
    const next = currentFunctions.includes(id)
      ? currentFunctions.filter((fn) => fn !== id)
      : [...currentFunctions, id]
    commitDraft(setFrontmatterStringList(current, 'functions', next))
  }
  const knownFunctionIds = new Set((functionCatalog.items ?? []).map((item) => item.id))
  const missingFunctions =
    functionCatalog.items === null ? [] : functions.filter((id) => !knownFunctionIds.has(id))
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

  return (
    <div
      className={`dir-ui-af-loading t-skel${catalogsLoading ? '' : ' is-revealed'}`}
      data-state={catalogsLoading ? 'loading' : 'loaded'}
      aria-busy={catalogsLoading}
    >
      <div className="t-skel-skeleton is-pulsing" aria-hidden={!catalogsLoading}>
        <AgentFormSkeletonLayout />
      </div>
      <div className="t-skel-content" aria-hidden={catalogsLoading} inert={catalogsLoading}>
        <div className="dir-ui-af">
          <div className="dir-ui-af-profile">
            <AvatarPicker
              icon={icon}
              color={color}
              readOnly={readOnly}
              onIconChange={setAvatar}
              onColorChange={setAvatarColor}
            />
            <div className="dir-ui-af-profile-copy">
              <div className="dir-ui-af-title-row">
                <InlineTextField
                  id={`${fieldId}-name`}
                  name="name"
                  value={nameValue}
                  placeholder="Untitled agent profile"
                  readOnly={readOnly}
                  onChange={setName}
                />
              </div>
              <InlineTextField
                id={`${fieldId}-description`}
                name="description"
                value={descriptionValue}
                placeholder="Add a concise description…"
                readOnly={readOnly}
                multiline
                onChange={setDescription}
              />
              {creating ? (
                <p className="dir-ui-af-file-hint">
                  {derived ? `${derived}.md` : 'The file name follows the agent profile name.'}
                </p>
              ) : null}
            </div>
          </div>

          <div className="dir-ui-af-aligned">
            {/* Profile settings: a two-up grid of stacked fields (label, hint,
                control) that fills the aligned column. The two pickers pair
                up; free text and the visibility switch span both tracks. */}
            <div className="dir-ui-af-settings">
              <div className="dir-ui-af-field">
                <div className="dir-ui-af-field-head">
                  <span className="dir-ui-af-field-label">Model</span>
                  <span className="dir-ui-af-field-hint">
                    {modelKnown ? 'Optional default.' : 'Unavailable in the catalog.'}
                  </span>
                </div>
                <div className="dir-ui-af-field-control">
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
                    className="dir-ui-af-field-picker"
                  />
                  {model && !readOnly ? (
                    <IconButton label="Use the session default model" onClick={() => setModel('')}>
                      <X />
                    </IconButton>
                  ) : null}
                </div>
              </div>

              <div className="dir-ui-af-field">
                <div className="dir-ui-af-field-head">
                  <span className="dir-ui-af-field-label">Extends</span>
                  <span className="dir-ui-af-field-hint">
                    {inheritanceError ? 'Saved chain does not resolve.' : 'Optional parent profile.'}
                  </span>
                </div>
                <div className="dir-ui-af-field-control">
                  <Select
                    className="dir-ui-af-field-picker"
                    aria-label="Parent agent profile"
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
                </div>
                {inheritanceError ? (
                  <p className="dir-ui-af-file-hint dir-ui-af-inheritance-error">{inheritanceError}</p>
                ) : null}
              </div>

              <div className="dir-ui-af-field is-wide">
                <div className="dir-ui-af-field-head">
                  <span className="dir-ui-af-field-label">
                    <label htmlFor={`${fieldId}-composer-placeholder`}>Composer example</label>
                  </span>
                  <span className="dir-ui-af-field-hint" id={`${fieldId}-composer-placeholder-hint`}>
                    Shown in an empty chat box for this profile. Never sent; not inherited.
                  </span>
                </div>
                <ComposerExampleField
                  id={`${fieldId}-composer-placeholder`}
                  value={composerPlaceholder}
                  readOnly={readOnly}
                  onChange={setComposerPlaceholder}
                />
              </div>

              <div className="dir-ui-af-field is-wide is-switch">
                <Switch
                  id={`${fieldId}-hidden`}
                  aria-describedby={`${fieldId}-hidden-hint`}
                  checked={hidden}
                  disabled={readOnly}
                  onChange={(event) => setHidden(event.currentTarget.checked)}
                />
                <div className="dir-ui-af-field-head">
                  <label className="dir-ui-af-field-label" htmlFor={`${fieldId}-hidden`}>
                    Hidden
                  </label>
                  <span className="dir-ui-af-field-hint" id={`${fieldId}-hidden-hint`}>
                    Keep out of the new-session gallery; still usable as a parent.
                  </span>
                </div>
              </div>
            </div>

            <CollapsibleSection
              title="System prompt"
              description="Instructions sessions using this profile follow — the whole identity, with nothing built-in underneath. Optional: with a parent, empty runs the parent's prompt alone; with no parent, empty means no identity prompt at all."
              summary="Markdown"
            >
              <SystemPromptEditor draft={draft} editDraft={editDraft} readOnly={readOnly} />
            </CollapsibleSection>

            <CollapsibleSection
              title="Skills"
              description="Move skills between the available and selected lists. Selected skills are preloaded into every session's prompt; empty = none."
              summary={`${skills.length} selected`}
            >
              <PickerEditor
                copy={SKILLS_COPY}
                items={skillCatalog.items}
                error={skillCatalog.error}
                selected={skills}
                missing={missingSkills}
                readOnly={readOnly}
                isSelected={isSkillSelected}
                onToggle={toggleSkill}
              />
            </CollapsibleSection>

            <CollapsibleSection
              title="Preloaded functions"
              description="Functions this profile uses routinely. Their contracts are pre-loaded into the system prompt of every new session, so the model calls them right away instead of searching and fetching each contract first."
              summary={`${functions.length} selected`}
            >
              <PickerEditor
                copy={FUNCTIONS_COPY}
                items={functionCatalog.items}
                error={functionCatalog.error}
                selected={functions}
                missing={missingFunctions}
                readOnly={readOnly}
                isSelected={isFunctionSelected}
                onToggle={toggleFunction}
              />
            </CollapsibleSection>

            {onRemove ? (
              <CollapsibleSection
                title="Danger area"
                description="Destructive actions for this agent profile."
                summary="Delete"
                defaultOpen={false}
              >
                <div className="dir-ui-af-remove-panel">
                  <div className="dir-ui-af-remove-copy">
                    <strong>Delete this agent profile</strong>
                    <span>This action cannot be undone.</span>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="dir-ui-af-remove"
                    disabled={saving || deleting}
                    onClick={onRemove}
                  >
                    {deleting ? 'Deleting…' : 'Delete agent profile'}
                  </Button>
                </div>
              </CollapsibleSection>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  )
}
