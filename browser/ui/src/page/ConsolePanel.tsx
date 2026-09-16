import {
  Button,
  EmptyState,
  type Host,
  SearchField,
  Select,
  StatusDot,
  StatusPanel,
  Toolbar,
} from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  BROWSER_CONSOLE_EVENT_TRIGGER,
  type BrowserConsoleEntry,
  errorMessage,
  formatTime,
  parseConsoleEvent,
  readBrowserConsole,
} from '../lib/browser'
import { cn } from '../lib/cn'
import { useBrowserSessionEvent } from '../lib/events'

/**
 * Live console feed for the selected session: seeded from
 * `browser::console::read`, then appended through the session-filtered
 * `browser::console-event` binding. The search input maps to the read's
 * `pattern` (regex, re-seeding on change); live entries are matched
 * client-side with the same expression so the feed stays consistent
 * between seeds.
 */

const SEED_LIMIT = 200
const MAX_ENTRIES = 500
const PATTERN_DEBOUNCE_MS = 300

const CONSOLE_FEED_FN = 'iii::browser-ui::console-feed'
const CONSOLE_LEVELS = ['all', 'debug', 'info', 'warning', 'error'] as const
type ConsoleLevel = (typeof CONSOLE_LEVELS)[number]

function matchesPattern(text: string, pattern: string): boolean {
  if (!pattern) return true
  try {
    return new RegExp(pattern, 'i').test(text)
  } catch {
    return text.toLowerCase().includes(pattern.toLowerCase())
  }
}

function matchesLevel(entryLevel: string, level: ConsoleLevel): boolean {
  if (level === 'all') return true
  if (level === 'error') return entryLevel === 'error' || entryLevel === 'exception'
  return entryLevel === level
}

function ConsoleLiveRow({ entry }: { entry: BrowserConsoleEntry }) {
  const tone =
    entry.level === 'error' || entry.level === 'exception' ? 'error' : entry.level === 'warning' ? 'warning' : 'info'
  return (
    <li className={cn('br-ui-devtools-row', `is-${tone}`)}>
      <StatusDot tone={tone === 'error' ? 'alert' : tone === 'warning' ? 'warn' : 'accent'} />
      <span className="br-ui-num br-ui-dim">{formatTime(entry.timestamp)}</span>
      <span className="br-ui-devtools-level">[{entry.level}]</span>
      <span className="br-ui-devtools-message">{entry.text}</span>
      <span className="br-ui-devtools-source">{entry.source ?? '—'}</span>
    </li>
  )
}

interface ConsolePanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function ConsolePanel({ host, sessionId, enabled }: ConsolePanelProps) {
  const [pattern, setPattern] = useState('')
  const [debouncedPattern, setDebouncedPattern] = useState('')
  const [level, setLevel] = useState<ConsoleLevel>('all')
  const [entries, setEntries] = useState<BrowserConsoleEntry[]>([])
  const [dropped, setDropped] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const lastSeqRef = useRef(0)
  const patternRef = useRef('')
  patternRef.current = debouncedPattern
  const levelRef = useRef<ConsoleLevel>('all')
  levelRef.current = level

  useEffect(() => {
    const id = window.setTimeout(() => setDebouncedPattern(pattern.trim()), PATTERN_DEBOUNCE_MS)
    return () => window.clearTimeout(id)
  }, [pattern])

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    lastSeqRef.current = 0
    setEntries([])
    void readBrowserConsole(host.iii, sessionId, {
      pattern: debouncedPattern || undefined,
      level: level === 'all' ? undefined : level,
      limit: SEED_LIMIT,
    })
      .then((res) => {
        if (cancelled || !res) return
        setEntries(res.entries)
        setDropped(res.dropped)
        lastSeqRef.current = res.last_seq
        setError(null)
      })
      .catch((err) => {
        if (cancelled) return
        setError(errorMessage(err))
      })
    return () => {
      cancelled = true
    }
  }, [host, enabled, sessionId, debouncedPattern, level])

  useBrowserSessionEvent({
    host,
    enabled,
    triggerType: BROWSER_CONSOLE_EVENT_TRIGGER,
    sessionId,
    fnId: CONSOLE_FEED_FN,
    onEvent: (payload) => {
      const evt = parseConsoleEvent(payload)
      if (!evt || evt.session_id !== sessionId) return
      if (evt.entry.seq <= lastSeqRef.current) return
      lastSeqRef.current = evt.entry.seq
      if (!matchesPattern(evt.entry.text, patternRef.current)) return
      if (!matchesLevel(evt.entry.level, levelRef.current)) return
      setEntries((cur) => [...cur.slice(-(MAX_ENTRIES - 1)), evt.entry])
    },
  })

  return (
    <div className="br-ui-panel">
      <Toolbar
        aria-label="console filters"
        end={
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setEntries([])
              setDropped(0)
            }}
          >
            Clear
          </Button>
        }
      >
        <span className="br-ui-panel-context">{sessionId}</span>
        <SearchField
          name="console-filter"
          value={pattern}
          onChange={setPattern}
          placeholder="filter (regex)"
          aria-label="filter console entries"
          className="br-ui-filter-input"
        />
        <Select<ConsoleLevel>
          value={level}
          onChange={setLevel}
          options={CONSOLE_LEVELS.map((option) => ({
            value: option,
            label: option === 'all' ? 'all levels' : option,
          }))}
          aria-label="console level"
          className="br-ui-level-select"
        />
        <span className="br-ui-panel-count">
          {entries.length} {entries.length === 1 ? 'entry' : 'entries'}
        </span>
        {dropped > 0 ? <span className="br-ui-panel-note">{dropped} older entries dropped from the buffer</span> : null}
      </Toolbar>
      {error ? (
        <div className="br-ui-panel-body">
          <StatusPanel variant="alert" headline="Console read failed" detail={error} />
        </div>
      ) : entries.length === 0 ? (
        <div className="br-ui-panel-body">
          <EmptyState
            title="No console entries yet"
            description="Messages the page logs appear here as they happen."
          />
        </div>
      ) : (
        // Column-reverse with the newest entry first in the DOM pins the
        // scroll position to the bottom, terminal-style.
        <ul className="br-ui-feed">
          {[...entries].reverse().map((entry) => (
            <ConsoleLiveRow key={entry.seq} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  )
}
