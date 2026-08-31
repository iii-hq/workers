import { Button, type Host, Input, type ModelOption, ModelPicker, Select } from '@iii-dev/console-ui'
import type { CSSProperties, KeyboardEvent, ReactNode } from 'react'
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { PencilIcon, SearchIcon, XIcon } from '../lib/widgets'
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
import { TokenIcon } from './token-icons'

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

function cssDurationMs(element: HTMLElement, variable: string, fallback: number) {
  const value = getComputedStyle(element).getPropertyValue(variable).trim()
  const duration = Number.parseFloat(value)
  if (!Number.isFinite(duration)) return fallback
  return value.endsWith('s') && !value.endsWith('ms') ? duration * 1000 : duration
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
  const triggerRef = useRef<HTMLButtonElement>(null)
  const closeTimerRef = useRef<number | null>(null)

  const close = useCallback(() => {
    if (state !== 'open') return
    setState('closing')
    if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current)
    const closeMs = wrapRef.current ? cssDurationMs(wrapRef.current, '--dropdown-close-dur', 150) : 150
    closeTimerRef.current = window.setTimeout(() => {
      setState('closed')
      closeTimerRef.current = null
    }, closeMs)
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

  useEffect(
    () => () => {
      if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current)
    },
    [],
  )

  const open = () => {
    if (readOnly) return
    if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current)
    setState('open')
  }

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
        onClick={() => (state === 'open' ? close() : open())}
      >
        <TokenIcon token={icon || 'agent'} size={24} />
      </button>
      <div
        data-origin="top-left"
        className={`dir-ui-af-avatar-pop t-dropdown${
          state === 'open' ? ' is-open' : state === 'closing' ? ' is-closing' : ''
        }`}
        role="dialog"
        aria-label="Agent profile avatars"
        aria-hidden={state === 'closed'}
        inert={state !== 'open'}
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
              <TokenIcon token={preset.token} size={20} />
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
            <XIcon className="dir-ui-af-avatar-option-icon" />
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
      <PencilIcon className="dir-ui-af-pencil" />
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
  const [open, setOpen] = useState(defaultOpen)
  const panelId = useId()
  return (
    <section className="dir-ui-af-disclosure t-acc" data-open={open}>
      <button
        type="button"
        className="dir-ui-af-disclosure-head t-acc-head"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((current) => !current)}
      >
        <span className="dir-ui-af-disclosure-copy">
          <span className="dir-ui-af-disclosure-title">{title}</span>
          <span className="dir-ui-af-disclosure-description">{description}</span>
        </span>
        {summary ? <span className="dir-ui-af-disclosure-summary">{summary}</span> : null}
        <span className="dir-ui-af-disclosure-chevron t-acc-chevron">
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <path d="M4 6.5L8 10.5L12 6.5" />
          </svg>
        </span>
      </button>
      <div id={panelId} className="t-acc-panel">
        <div className="dir-ui-af-disclosure-inner t-acc-panel-inner">
          <div className="dir-ui-af-disclosure-content">{children}</div>
        </div>
      </div>
    </section>
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

function SkillCheckbox({
  item,
  checked,
  disabled,
  missing = false,
  onChange,
}: {
  item: PickItem
  checked: boolean
  disabled: boolean
  missing?: boolean
  onChange: () => void
}) {
  return (
    <label className="dir-ui-af-skill-row">
      <input
        type="checkbox"
        name="skills"
        value={item.id}
        checked={checked}
        disabled={disabled}
        className="dir-ui-af-skill-native"
        onChange={onChange}
      />
      <span
        className="dir-ui-af-skill-check t-check"
        aria-hidden="true"
        data-checked={checked}
        style={{ '--check-len': 15 } as CSSProperties}
      >
        <svg viewBox="0 0 10.1668 10.1668" aria-hidden="true">
          <path d="M1 5.52L3.92 9.17L9.17 1" />
        </svg>
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

function SkillList({
  title,
  items,
  checked,
  disabled,
  empty,
  onToggle,
}: {
  title: string
  items: (PickItem & { missing?: boolean })[]
  checked: boolean
  disabled: boolean
  empty: string
  onToggle: (id: string) => void
}) {
  return (
    <div className="dir-ui-af-skill-list-wrap">
      <div className="dir-ui-af-skill-list-head">
        <span>{title}</span>
        <span className="dir-ui-af-skill-count">{items.length}</span>
      </div>
      {/* The explicit role keeps list semantics when host styles remove markers. */}
      {/* biome-ignore lint/a11y/noRedundantRoles: preserve list semantics across embedded hosts */}
      <ul className="dir-ui-af-skill-list" role="list">
        {items.length === 0 ? (
          <li className="dir-ui-af-skill-empty">{empty}</li>
        ) : (
          items.map((item) => (
            <li key={item.id}>
              <SkillCheckbox
                item={item}
                checked={checked}
                disabled={disabled}
                missing={item.missing}
                onChange={() => onToggle(item.id)}
              />
            </li>
          ))
        )}
      </ul>
    </div>
  )
}

function SkillsEditor({
  items,
  error,
  selected,
  missing,
  readOnly,
  isSelected,
  onToggle,
}: {
  items: PickItem[] | null
  error: boolean
  selected: string[]
  missing: string[]
  readOnly: boolean
  isSelected: (id: string) => boolean
  onToggle: (id: string) => void
}) {
  const [filter, setFilter] = useState('')
  const needle = filter.trim().toLowerCase()
  const matches = (item: PickItem) =>
    !needle ||
    item.id.toLowerCase().includes(needle) ||
    item.label.toLowerCase().includes(needle) ||
    (item.desc ?? '').toLowerCase().includes(needle)

  if (error) {
    return (
      <div className="dir-ui-af-catalog-message" role="status">
        The skill catalog could not be loaded. Existing selections will be preserved when you save.
      </div>
    )
  }
  if (items === null) {
    return <div className="dir-ui-af-catalog-message">Loading skills…</div>
  }

  const available = items.filter((item) => !isSelected(item.id) && matches(item))
  const selectedItems: (PickItem & { missing?: boolean })[] = [
    ...items.filter((item) => isSelected(item.id) && matches(item)),
    ...missing
      .map((id) => ({
        id,
        label: id,
        desc: 'This skill is not in the current catalog.',
        missing: true,
      }))
      .filter(matches),
  ]

  return (
    <div className="dir-ui-af-skills">
      <div className="dir-ui-af-skill-search">
        <SearchIcon className="dir-ui-af-skill-search-icon" />
        <Input
          type="search"
          name="skill_filter"
          value={filter}
          onChange={setFilter}
          placeholder="Filter skills…"
          aria-label="Filter skills"
          spellCheck={false}
          className="dir-ui-af-skill-search-input"
        />
        {filter ? (
          <button
            type="button"
            className="dir-ui-af-skill-search-clear"
            aria-label="Clear skill filter"
            onClick={() => setFilter('')}
          >
            <XIcon className="dir-ui-af-skill-search-clear-icon" />
          </button>
        ) : null}
      </div>
      <SkillList
        title="Selected"
        items={selectedItems}
        checked
        disabled={readOnly}
        empty={
          needle
            ? 'No selected skills match.'
            : selected.length === 0
              ? 'No filter — sessions using this profile can use every skill.'
              : 'No skills selected.'
        }
        onToggle={onToggle}
      />
      <div className="dir-ui-af-skill-transfer" aria-hidden="true">
        <span>↓</span>
        <span>↑</span>
      </div>
      <SkillList
        title="Available"
        items={available}
        checked={false}
        disabled={readOnly}
        empty={needle ? 'No available skills match.' : 'All skills selected.'}
        onToggle={onToggle}
      />
    </div>
  )
}

function SkeletonSkillList() {
  return (
    <div className="dir-ui-af-skill-list-wrap">
      <div className="dir-ui-af-skill-list-head">
        <span className="dir-ui-af-skeleton-block is-list-title" />
        <span className="dir-ui-af-skeleton-block is-count" />
      </div>
      {/* biome-ignore lint/a11y/noRedundantRoles: preserve list semantics across embedded hosts */}
      <ul className="dir-ui-af-skill-list" role="list">
        {[0, 1].map((row) => (
          <li key={row}>
            <div className="dir-ui-af-skeleton-skill-row">
              <span className="dir-ui-af-skeleton-block is-checkbox" />
              <span className="dir-ui-af-skill-copy">
                <span className="dir-ui-af-skeleton-block is-skill-name" />
                <span className="dir-ui-af-skeleton-block is-skill-description" />
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
    <section className="dir-ui-af-disclosure dir-ui-af-skeleton-disclosure">
      <div className="dir-ui-af-disclosure-head">
        <span className="dir-ui-af-disclosure-copy">
          <span className="dir-ui-af-skeleton-block is-section-title" />
          <span className="dir-ui-af-skeleton-block is-section-description" />
        </span>
        <span className="dir-ui-af-disclosure-summary dir-ui-af-skeleton-block is-summary" />
        <span className="dir-ui-af-disclosure-chevron dir-ui-af-skeleton-block is-chevron" />
      </div>
      <div className="dir-ui-af-disclosure-content">
        {kind === 'prompt' ? (
          <div className="dir-ui-af-prompt dir-ui-af-skeleton-prompt">
            <span className="dir-ui-af-skeleton-block is-prompt-line" />
            <span className="dir-ui-af-skeleton-block is-prompt-line is-medium" />
            <span className="dir-ui-af-skeleton-block is-prompt-line is-short" />
          </div>
        ) : (
          <div className="dir-ui-af-skills">
            <span className="dir-ui-af-skeleton-block is-search" />
            <SkeletonSkillList />
            <span className="dir-ui-af-skeleton-block is-transfer" />
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
        <span className="dir-ui-af-skeleton-block is-avatar" />
        <div className="dir-ui-af-profile-copy">
          <span className="dir-ui-af-skeleton-block is-name" />
          <span className="dir-ui-af-skeleton-block is-description" />
          <span className="dir-ui-af-skeleton-block is-description-short" />
        </div>
      </div>

      <div className="dir-ui-af-aligned">
        <div className="dir-ui-af-model-row">
          <div className="dir-ui-af-model-label">
            <span className="dir-ui-af-skeleton-block is-model-label" />
            <span className="dir-ui-af-skeleton-block is-model-hint" />
          </div>
          <span className="dir-ui-af-skeleton-block is-model-picker" />
        </div>
        <SkeletonDisclosure kind="prompt" />
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
  const extendsId = readFrontmatterField(draft, ['extends']).value.trim()
  const model = readFrontmatterField(draft, ['model']).value.trim()
  const reasoningEffort = readFrontmatterField(draft, ['reasoning_effort']).value.trim() || 'default'
  const icon = readFrontmatterField(draft, ['icon']).value.trim()
  const color = readFrontmatterField(draft, ['color']).value.trim()
  const skillCatalog = useCatalog(host, fetchSkills)
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
    (skillCatalog.items === null && !skillCatalog.error) || (modelCatalog.items === null && !modelCatalog.error)

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
            <div className="dir-ui-af-model-row">
              <div className="dir-ui-af-model-label">
                <span>Model</span>
                <span>{modelKnown ? 'Optional default.' : 'Unavailable in the catalog.'}</span>
              </div>
              <div className="dir-ui-af-model-control">
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
                  className="dir-ui-af-model-picker"
                />
                {model && !readOnly ? (
                  <button
                    type="button"
                    className="dir-ui-af-model-clear"
                    aria-label="Use the session default model"
                    title="Use the session default"
                    onClick={() => setModel('')}
                  >
                    <XIcon className="dir-ui-af-model-clear-icon" />
                  </button>
                ) : null}
              </div>
            </div>

            <div className="dir-ui-af-model-row">
              <div className="dir-ui-af-model-label">
                <span>Extends</span>
                <span>{inheritanceError ? 'Saved chain does not resolve.' : 'Optional parent profile.'}</span>
              </div>
              <div className="dir-ui-af-model-control">
                <Select
                  className="dir-ui-af-model-picker"
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
            </div>
            {inheritanceError ? (
              <p className="dir-ui-af-file-hint dir-ui-af-inheritance-error">{inheritanceError}</p>
            ) : null}

            <CollapsibleSection
              title="System prompt"
              description="Instructions sessions using this profile follow. With a parent, they are appended after the parent's resolved prompt."
              summary="Markdown"
            >
              <SystemPromptEditor draft={draft} editDraft={editDraft} readOnly={readOnly} />
            </CollapsibleSection>

            <CollapsibleSection
              title="Skills"
              description="Move skills between the available and selected lists."
              summary={`${skills.length} selected`}
            >
              <SkillsEditor
                items={skillCatalog.items}
                error={skillCatalog.error}
                selected={skills}
                missing={missingSkills}
                readOnly={readOnly}
                isSelected={isSkillSelected}
                onToggle={toggleSkill}
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
