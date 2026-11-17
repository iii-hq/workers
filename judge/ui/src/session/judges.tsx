import { IconButton } from '@iii-dev/console-ui'
import { Check, Plus, Scale, Search, Settings2, TriangleAlert } from 'lucide-react'
import { type KeyboardEvent, type ReactNode, useEffect, useId, useState } from 'react'
import type { JudgeProvider, JudgeSettings } from './engine'

const FILTER_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']

/** One worker's settings, opened on the picker's settings page. */
export interface ConfigureTarget {
  configurationId: string
  title: string
  description: string
}

interface JudgeRow {
  /** Stable and selector-safe: `default` or the provider name. */
  id: string
  /** What choosing it writes to the session; `undefined` = Default. */
  value: string | undefined
  mark: ReactNode
  /** The model the judge's settings name (or the provider, failing that). */
  label: string
  /** The provider, beside the label; the Default row carries none. */
  meta?: string
  description?: string
  /** The description says why this choice would not be used. */
  warning?: boolean
  configure?: ConfigureTarget
}

/** A provider's glyph: the registry publishes no icons, so its initial. */
export function JudgeMark({ provider }: { provider: string }) {
  return (
    <span className="judge-ui-session-mark" aria-hidden="true">
      {provider.charAt(0).toUpperCase()}
    </span>
  )
}

export function providerTarget(provider: JudgeProvider): ConfigureTarget | undefined {
  return provider.configurationId
    ? {
        configurationId: provider.configurationId,
        title: provider.provider,
        description: `Credentials, model and limits of ${provider.worker}.`,
      }
    : undefined
}

export function settingsTarget(settings: JudgeSettings): ConfigureTarget {
  return {
    configurationId: settings.configurationId,
    title: 'Judge settings',
    description: 'The default judge and how local judges load.',
  }
}

/**
 * The picker's choices, one flat list the way the model picker lists
 * models: the judge settings' Default first, then one row per provider
 * showing the model its settings name, each with its own Configure.
 */
function judgeRows(providers: JudgeProvider[] | null, settings: JudgeSettings | null, stored: string | undefined): JudgeRow[] {
  const running = new Map((providers ?? []).map((entry) => [entry.provider, entry]))
  const fallback = settings?.provider
  const fallbackEntry = fallback ? running.get(fallback) : undefined
  const fallbackDescription = !fallback
    ? 'From judge settings'
    : providers && !fallbackEntry
      ? `${fallback}, not running`
      : fallbackEntry?.problem
        ? `${fallback}, not configured`
        : [fallback, fallbackEntry?.model].filter(Boolean).join(' · ')
  const names = [...running.keys()]
  if (stored && providers && !running.has(stored)) names.push(stored)
  return [
    {
      id: 'default',
      value: undefined,
      mark: <Scale size={16} aria-hidden />,
      label: 'Default',
      description: fallbackDescription,
      warning: Boolean(fallback && providers && (!fallbackEntry || fallbackEntry.problem)),
      configure: settings ? settingsTarget(settings) : undefined,
    },
    ...names.sort().map((name): JudgeRow => {
      const entry = running.get(name)
      return {
        id: name,
        value: name,
        mark: <JudgeMark provider={name} />,
        label: entry?.model ?? name,
        meta: entry?.model ? name : undefined,
        description: !entry ? 'Not running' : entry.problem ? `Not configured: ${entry.problem}` : undefined,
        warning: !entry || Boolean(entry.problem),
        configure: entry ? providerTarget(entry) : undefined,
      }
    }),
  ]
}

export interface JudgesPanelProps {
  /** `null` until the first listing answers. */
  providers: JudgeProvider[] | null
  settings: JudgeSettings | null
  /** The session's choice; `undefined` = Default. */
  stored: string | undefined
  listError: string | null
  onChoose(value: string | undefined): void
  onConfigure(target: ConfigureTarget): void
  onAdd(): void
}

/**
 * The first page: a filter you type into (arrows move, Enter picks, as in
 * the model picker), the rows, and a foot with the judge settings and the
 * way to add a judge.
 */
export function JudgesPanel({ providers, settings, stored, listError, onChoose, onConfigure, onAdd }: JudgesPanelProps) {
  const listId = useId()
  const [filter, setFilter] = useState('')
  // `null` follows the session's choice; typing or hovering pins a row.
  const [activeIndex, setActiveIndex] = useState<number | null>(null)

  const words = filter.toLowerCase().split(/\s+/).filter(Boolean)
  const visible = judgeRows(providers, settings, stored).filter((row) => {
    if (words.length === 0) return true
    const hay = `${row.meta ?? ''} ${row.label} ${row.description ?? ''} ${row.value ?? 'default'}`.toLowerCase()
    return words.every((word) => hay.includes(word))
  })
  const selectedIndex = visible.findIndex((row) => row.value === stored)
  const resolvedIndex = activeIndex ?? Math.max(0, selectedIndex)
  const active = visible[Math.min(resolvedIndex, visible.length - 1)]
  const optionId = (row: JudgeRow) => `${listId}-${row.id}`

  useEffect(() => {
    if (!active) return
    document.getElementById(optionId(active))?.scrollIntoView?.({ block: 'nearest' })
  }, [active, optionId])

  const onFilterKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // The menu would take every key for its own navigation; Escape reaches
    // it only once the filter is empty, to close the picker.
    if (event.key !== 'Escape' || filter !== '') event.stopPropagation()
    if (!FILTER_KEYS.includes(event.key)) return
    if (event.key === 'Escape') {
      if (filter === '') return
      event.preventDefault()
      setFilter('')
      setActiveIndex(null)
      return
    }
    event.preventDefault()
    if (event.key === 'Enter') {
      if (active) onChoose(active.value)
      return
    }
    if (visible.length === 0) return
    if (event.key === 'Home' || event.key === 'End') {
      setActiveIndex(event.key === 'Home' ? 0 : visible.length - 1)
      return
    }
    const step = event.key === 'ArrowDown' ? 1 : -1
    setActiveIndex((Math.min(resolvedIndex, visible.length - 1) + step + visible.length) % visible.length)
  }

  return (
    <>
      <div className="judge-ui-session-filter">
        <Search size={16} aria-hidden />
        <input
          type="search"
          value={filter}
          onChange={(event) => {
            setFilter(event.target.value)
            setActiveIndex(0)
          }}
          onKeyDown={onFilterKeyDown}
          // biome-ignore lint/a11y/noAutofocus: the menu opened to pick a judge; typing is the fastest way to one
          autoFocus
          placeholder="Filter judges…"
          aria-label="Filter judges"
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={active ? optionId(active) : undefined}
          autoCapitalize="none"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
        />
      </div>
      <div id={listId} className="judge-ui-session-list">
        {visible.map((row) => {
          const selected = row.value === stored
          return (
            <div key={row.id} className="judge-ui-session-row">
              <button
                id={optionId(row)}
                data-judge-option={row.id}
                type="button"
                className="judge-ui-session-option"
                aria-pressed={selected}
                data-selected={selected || undefined}
                data-highlighted={row === active || undefined}
                data-warning={row.warning || undefined}
                title={row.warning ? row.description : undefined}
                onClick={() => onChoose(row.value)}
                onMouseEnter={() => setActiveIndex(visible.indexOf(row))}
              >
                {row.mark}
                <span className="judge-ui-session-option-copy">
                  <span className="judge-ui-session-option-title">
                    <span className="judge-ui-session-option-label">{row.label}</span>
                    {row.meta ? <span className="judge-ui-session-option-meta">{row.meta}</span> : null}
                  </span>
                  {row.description ? (
                    <span className="judge-ui-session-option-description">
                      {row.warning ? <TriangleAlert size={16} aria-hidden /> : null}
                      {row.description}
                    </span>
                  ) : null}
                </span>
                <Check size={16} aria-hidden className="judge-ui-session-option-check" />
              </button>
              {row.configure ? (
                <IconButton
                  label={`Configure ${row.id === 'default' ? 'judge settings' : row.id}`}
                  tooltipSide="left"
                  variant="ghost"
                  className="judge-ui-session-row-configure"
                  onClick={() => row.configure && onConfigure(row.configure)}
                >
                  <Settings2 size={16} aria-hidden />
                </IconButton>
              ) : null}
            </div>
          )
        })}
        {visible.length === 0 && words.length > 0 ? (
          <p className="judge-ui-session-empty">No judge matches “{filter.trim()}”.</p>
        ) : null}
        {providers === null && words.length === 0 ? (
          <p className="judge-ui-session-empty" role="status">
            Checking judges…
          </p>
        ) : null}
        {providers !== null && providers.length === 0 && words.length === 0 ? (
          <p className="judge-ui-session-empty">No judge providers yet. Add one from the registry.</p>
        ) : null}
        {listError ? (
          <p className="judge-ui-session-empty" role="alert">
            Could not list judges: {listError}
          </p>
        ) : null}
      </div>
      <div className="judge-ui-session-foot">
        <span className="judge-ui-session-foot-note">Applies from the next turn</span>
        <button type="button" className="judge-ui-session-foot-action" onClick={onAdd}>
          <Plus size={16} aria-hidden />
          Add a judge
        </button>
      </div>
    </>
  )
}
