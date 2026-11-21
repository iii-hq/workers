import { IconButton } from '@iii-dev/console-ui'
import { Check, CircleOff, Plus, Scale, Search, Settings2, TriangleAlert } from 'lucide-react'
import { type KeyboardEvent, type ReactNode, useEffect, useId, useState } from 'react'
import type { JudgeProvider, JudgeSettings } from './engine'

const FILTER_KEYS = ['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', 'Escape']
/** Under this many choices a filter is clutter: the eye finds a row faster. */
const FILTER_FROM = 6

/**
 * The session value that turns the judge off: the hub answers it as "no
 * judge" without forwarding, and every caller carries on as it does when no
 * judge is deployed (`judge_contract::OFF_PROVIDER`).
 */
export const JUDGE_OFF = 'off'

/** One worker's settings, opened on the picker's settings page. */
export interface ConfigureTarget {
  configurationId: string
  title: string
  description: string
}

interface JudgeRow {
  /** Stable and selector-safe: `default`, `off` or the provider name. */
  id: string
  /** What choosing it writes to the session; `undefined` = Default. */
  value: string | undefined
  mark: ReactNode
  label: string
  /** One quiet line under the name: the model, or what is wrong. */
  description?: string
  /** The description says why this choice would not be used. */
  warning?: boolean
  /** The full reason behind a warning, with its fix. */
  reason?: string
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

/** What is wrong, in the words of the fix: the engine's reason is the title. */
function problemLabel(problem: string): string {
  return /api key/i.test(problem) ? 'Needs an API key' : 'Not configured'
}

/**
 * The picker's choices, one flat list: the judge settings' Default first,
 * then one row per provider with the model its settings name.
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
  const fallbackWarning = Boolean(fallback && providers && (!fallbackEntry || fallbackEntry.problem))
  const names = [...running.keys()].filter((name) => name !== JUDGE_OFF)
  if (stored && stored !== JUDGE_OFF && providers && !running.has(stored)) names.push(stored)
  return [
    {
      id: 'default',
      value: undefined,
      mark: (
        <span className="judge-ui-session-mark" aria-hidden="true">
          <Scale size={16} aria-hidden />
        </span>
      ),
      label: 'Default',
      description: fallbackDescription,
      warning: fallbackWarning,
      reason: fallbackWarning
        ? fallbackEntry?.problem
          ? `${fallback} is not configured: ${fallbackEntry.problem}`
          : `judge-${fallback} is not running`
        : undefined,
    },
    {
      id: JUDGE_OFF,
      value: JUDGE_OFF,
      mark: (
        <span className="judge-ui-session-mark" aria-hidden="true">
          <CircleOff size={16} aria-hidden />
        </span>
      ),
      label: 'Off',
      description: 'No judge in this session',
    },
    ...names.sort().map((name): JudgeRow => {
      const entry = running.get(name)
      return {
        id: name,
        value: name,
        mark: <JudgeMark provider={name} />,
        label: name,
        description: !entry ? 'Not running' : entry.problem ? problemLabel(entry.problem) : (entry.model ?? undefined),
        warning: !entry || Boolean(entry.problem),
        reason: !entry ? `judge-${name} is not running` : entry.problem ? `Not configured: ${entry.problem}` : undefined,
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
  /** The one-time note on what a judge is, under the head. */
  intro?: ReactNode
}

/**
 * The first page: a head naming what this is and when it applies, with the
 * judge settings; the rows; a filter once there are enough rows to need
 * one (arrows move, Enter picks); and the way to add a judge.
 */
export function JudgesPanel({
  providers,
  settings,
  stored,
  listError,
  onChoose,
  onConfigure,
  onAdd,
  intro,
}: JudgesPanelProps) {
  const listId = useId()
  const [filter, setFilter] = useState('')
  // `null` follows the session's choice; typing or hovering pins a row.
  const [activeIndex, setActiveIndex] = useState<number | null>(null)

  const rows = judgeRows(providers, settings, stored)
  const showFilter = rows.length >= FILTER_FROM
  const words = showFilter ? filter.toLowerCase().split(/\s+/).filter(Boolean) : []
  const visible = rows.filter((row) => {
    if (words.length === 0) return true
    const hay = `${row.label} ${row.description ?? ''} ${row.value ?? 'default'}`.toLowerCase()
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
      <div className="judge-ui-session-head">
        <div className="judge-ui-session-head-copy">
          <span className="judge-ui-session-head-title">Judge</span>
          <span className="judge-ui-session-head-note">Applies from the next turn</span>
        </div>
        {settings ? (
          <IconButton
            label="Configure judge settings"
            tooltip="Judge settings"
            tooltipSide="top"
            variant="ghost"
            className="judge-ui-session-head-settings"
            onClick={() => onConfigure(settingsTarget(settings))}
          >
            <Settings2 size={16} aria-hidden />
          </IconButton>
        ) : null}
      </div>
      {intro}
      {showFilter ? (
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
      ) : null}
      <div id={listId} className="judge-ui-session-list">
        {visible.map((row) => {
          const selected = row.value === stored
          const tip = row.reason ? (
            <span className="judge-ui-session-tip">
              <span>{row.reason}</span>
              {row.configure ? <span className="judge-ui-session-tip-action">Configure {row.id}</span> : null}
            </span>
          ) : undefined
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
                title={row.reason}
                onClick={() => onChoose(row.value)}
                onMouseEnter={() => setActiveIndex(visible.indexOf(row))}
              >
                {row.mark}
                <span className="judge-ui-session-option-copy">
                  <span className="judge-ui-session-option-label">{row.label}</span>
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
                  label={`Configure ${row.id}`}
                  // A warning's whole reason rides on the settings button's
                  // tooltip, so the keyboard reaches it, not only a pointer.
                  tooltip={tip}
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
          <p className="judge-ui-session-empty">No judge yet. Add one from the registry below.</p>
        ) : null}
        {listError ? (
          <p className="judge-ui-session-empty" role="alert">
            Could not list judges: {listError}
          </p>
        ) : null}
      </div>
      <div className="judge-ui-session-foot">
        <button type="button" className="judge-ui-session-foot-action" onClick={onAdd}>
          <Plus size={16} aria-hidden />
          Add a judge
        </button>
      </div>
    </>
  )
}
