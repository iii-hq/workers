import { IconButton } from '@iii-dev/console-ui'
import { Check, Plus, Scale } from 'lucide-react'
import { type KeyboardEvent, type ReactNode, useCallback, useEffect, useId, useRef, useState } from 'react'
import type { JudgeProvider, JudgeSettings } from './engine'

/** After a rail tap the scroll spy stays quiet until the smooth scroll lands. */
const RAIL_JUMP_SETTLE_MS = 700
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
  label: string
  description?: string
}

interface JudgeGroup {
  key: string
  label: string
  mark: ReactNode
  configure?: ConfigureTarget
  rows: JudgeRow[]
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
 * The picker's choices, grouped the way the model picker groups models: the
 * judge settings' Default first, then one group per provider showing the
 * model its settings name. Each group heading has its Configure.
 */
function judgeGroups(
  providers: JudgeProvider[] | null,
  settings: JudgeSettings | null,
  stored: string | undefined,
): JudgeGroup[] {
  const running = new Map((providers ?? []).map((entry) => [entry.provider, entry]))
  const fallback = settings?.provider
  const fallbackDescription = !fallback
    ? 'From judge settings'
    : providers && !running.has(fallback)
      ? `${fallback}, not running`
      : [fallback, running.get(fallback)?.model].filter(Boolean).join(' · ')
  const names = [...running.keys()]
  if (stored && providers && !running.has(stored)) names.push(stored)
  return [
    {
      key: 'default',
      label: 'Judge settings',
      mark: <Scale size={16} aria-hidden />,
      configure: settings ? settingsTarget(settings) : undefined,
      rows: [{ id: 'default', value: undefined, label: 'Default', description: fallbackDescription }],
    },
    ...names.sort().map((name) => {
      const entry = running.get(name)
      return {
        key: name,
        label: name,
        mark: <JudgeMark provider={name} />,
        configure: entry ? providerTarget(entry) : undefined,
        rows: [
          {
            id: name,
            value: name,
            label: entry?.model ?? name,
            description: entry ? undefined : 'Not running',
          },
        ],
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
 * the model picker), a rail with one glyph per group and the add button,
 * and the grouped choices.
 */
export function JudgesPanel({ providers, settings, stored, listError, onChoose, onConfigure, onAdd }: JudgesPanelProps) {
  const listId = useId()
  const [filter, setFilter] = useState('')
  // `null` follows the session's choice; typing or hovering pins a row.
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const [activeGroup, setActiveGroup] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const scrollFrameRef = useRef<number | null>(null)
  const railJumpRef = useRef<{ key: string; until: number } | null>(null)

  const words = filter.toLowerCase().split(/\s+/).filter(Boolean)
  const groups = judgeGroups(providers, settings, stored)
    .map((group) => ({
      ...group,
      rows: group.rows.filter((row) => {
        if (words.length === 0) return true
        const hay = `${group.label} ${row.label} ${row.description ?? ''} ${row.value ?? 'default'}`.toLowerCase()
        return words.every((word) => hay.includes(word))
      }),
    }))
    .filter((group) => words.length === 0 || group.rows.length > 0)
  const groupsKey = groups.map((group) => group.key).join(' ')
  const visible = groups.flatMap((group) => group.rows)
  const selectedIndex = visible.findIndex((row) => row.value === stored)
  const resolvedIndex = activeIndex ?? Math.max(0, selectedIndex)
  const active = visible[Math.min(resolvedIndex, visible.length - 1)]
  const optionId = (row: JudgeRow) => `${listId}-${row.id}`

  useEffect(() => {
    if (!active) return
    listRef.current?.querySelector(`[data-judge-option="${active.id}"]`)?.scrollIntoView?.({ block: 'nearest' })
  }, [active])

  // The rail highlights the group whose heading sits at (or just above) the
  // top of the list — or the last one once the list is scrolled to the end.
  const syncActiveGroup = useCallback(() => {
    const list = listRef.current
    if (!list) return
    const pinned = railJumpRef.current
    if (pinned && Date.now() < pinned.until) {
      setActiveGroup(pinned.key)
      return
    }
    railJumpRef.current = null
    const sections = Array.from(list.querySelectorAll<HTMLElement>('[data-judge-group]'))
    if (sections.length === 0) {
      setActiveGroup(null)
      return
    }
    const scrollable = list.scrollHeight > list.clientHeight + 2
    const atEnd = scrollable && list.scrollTop + list.clientHeight >= list.scrollHeight - 2
    let current = sections[0]
    if (atEnd) current = sections[sections.length - 1]
    else {
      for (const section of sections) {
        if (section.offsetTop <= list.scrollTop + 16) current = section
        else break
      }
    }
    setActiveGroup(current.dataset.judgeGroup ?? null)
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: the spy reads the DOM, so it re-runs whenever the rendered groups change
  useEffect(() => {
    syncActiveGroup()
  }, [syncActiveGroup, groupsKey])

  useEffect(
    () => () => {
      if (scrollFrameRef.current !== null) cancelAnimationFrame(scrollFrameRef.current)
    },
    [],
  )

  const onListScroll = () => {
    if (scrollFrameRef.current !== null) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null
      syncActiveGroup()
    })
  }

  const jumpToGroup = (key: string) => {
    const list = listRef.current
    const section = list?.querySelector<HTMLElement>(`[data-judge-group="${key}"]`)
    if (!list || !section) return
    const reduceMotion =
      typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
    railJumpRef.current = { key, until: Date.now() + RAIL_JUMP_SETTLE_MS }
    setActiveGroup(key)
    list.scrollTo?.({ top: section.offsetTop, behavior: reduceMotion ? 'auto' : 'smooth' })
  }

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
      <div className="judge-ui-session-browse">
        <nav aria-label="Judges" className="judge-ui-session-rail">
          {groups.map((group) => (
            <IconButton
              key={group.key}
              label={group.key === 'default' ? 'Default' : group.label}
              tooltipSide="right"
              variant="ghost"
              className="judge-ui-session-rail-button"
              aria-current={group.key === activeGroup ? 'true' : undefined}
              data-judge-rail={group.key}
              onClick={() => jumpToGroup(group.key)}
            >
              {group.mark}
            </IconButton>
          ))}
          <IconButton
            label="Add a judge"
            tooltipSide="right"
            variant="ghost"
            className="judge-ui-session-rail-button"
            onClick={onAdd}
          >
            <Plus size={16} aria-hidden />
          </IconButton>
        </nav>
        <div id={listId} ref={listRef} onScroll={onListScroll} className="judge-ui-session-list">
          {groups.length === 0 ? (
            <p className="judge-ui-session-empty">No judge matches “{filter.trim()}”.</p>
          ) : (
            groups.map((group) => (
              <section key={group.key} aria-label={group.label} data-judge-group={group.key}>
                <div className="judge-ui-session-group-head">
                  <h3 className="judge-ui-session-group-label">
                    {group.mark}
                    <span>{group.label}</span>
                  </h3>
                  {group.configure ? (
                    <button
                      type="button"
                      className="judge-ui-session-configure"
                      aria-label={`Configure ${group.key === 'default' ? 'judge settings' : group.label}`}
                      onClick={() => group.configure && onConfigure(group.configure)}
                    >
                      Configure
                    </button>
                  ) : null}
                </div>
                <div className="judge-ui-session-card">
                  {group.rows.map((row) => {
                    const selected = row.value === stored
                    return (
                      <button
                        key={row.id}
                        id={optionId(row)}
                        data-judge-option={row.id}
                        type="button"
                        className="judge-ui-session-option"
                        aria-pressed={selected}
                        data-selected={selected || undefined}
                        data-highlighted={row === active || undefined}
                        onClick={() => onChoose(row.value)}
                        onMouseEnter={() => setActiveIndex(visible.indexOf(row))}
                      >
                        <span className="judge-ui-session-option-copy">
                          <span className="judge-ui-session-option-label">{row.label}</span>
                          {row.description ? (
                            <span className="judge-ui-session-option-description">{row.description}</span>
                          ) : null}
                        </span>
                        {selected ? <Check size={16} aria-hidden /> : null}
                      </button>
                    )
                  })}
                </div>
              </section>
            ))
          )}
          {providers === null && words.length === 0 ? (
            <p className="judge-ui-session-empty" role="status">
              Checking judges…
            </p>
          ) : null}
          {providers !== null && providers.length === 0 && words.length === 0 ? (
            <p className="judge-ui-session-empty">No judge providers yet. Add one from the registry with +.</p>
          ) : null}
          {listError ? (
            <p className="judge-ui-session-empty" role="alert">
              Could not list judges: {listError}
            </p>
          ) : null}
        </div>
      </div>
    </>
  )
}
