import { Input, type Host } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import {
  BROWSER_CONSOLE_EVENT_TRIGGER,
  type BrowserConsoleEntry,
  errorMessage,
  parseConsoleEvent,
  readBrowserConsole,
} from '../lib/browser'
import { useBrowserSessionEvent } from '../lib/events'
import { ConsoleEntryRow } from '../function-trigger-message/BrowserViews'

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

function matchesPattern(text: string, pattern: string): boolean {
  if (!pattern) return true
  try {
    return new RegExp(pattern, 'i').test(text)
  } catch {
    return text.toLowerCase().includes(pattern.toLowerCase())
  }
}

interface ConsolePanelProps {
  host: Host
  sessionId: string
  enabled: boolean
}

export function ConsolePanel({ host, sessionId, enabled }: ConsolePanelProps) {
  const [pattern, setPattern] = useState('')
  const [debouncedPattern, setDebouncedPattern] = useState('')
  const [entries, setEntries] = useState<BrowserConsoleEntry[]>([])
  const [dropped, setDropped] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const lastSeqRef = useRef(0)
  const patternRef = useRef('')
  patternRef.current = debouncedPattern

  useEffect(() => {
    const id = window.setTimeout(
      () => setDebouncedPattern(pattern.trim()),
      PATTERN_DEBOUNCE_MS,
    )
    return () => window.clearTimeout(id)
  }, [pattern])

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    lastSeqRef.current = 0
    setEntries([])
    void readBrowserConsole(host.iii, sessionId, {
      pattern: debouncedPattern || undefined,
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
  }, [host, enabled, sessionId, debouncedPattern])

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
      setEntries((cur) => [...cur.slice(-(MAX_ENTRIES - 1)), evt.entry])
    },
  })

  return (
    <div className="br-ui-panel">
      <div className="br-ui-panel-head">
        <Input
          value={pattern}
          onChange={setPattern}
          preserveCase
          placeholder="filter (regex)"
          aria-label="filter console entries"
          className="br-ui-filter-input"
        />
        {dropped > 0 ? (
          <span className="br-ui-panel-note">
            {dropped} older entries dropped from the buffer
          </span>
        ) : null}
      </div>
      {error ? (
        <p className="br-ui-panel-err">{error}</p>
      ) : entries.length === 0 ? (
        <p className="br-ui-panel-empty">no console entries yet</p>
      ) : (
        // Column-reverse with the newest entry first in the DOM pins the
        // scroll position to the bottom, terminal-style.
        <ul className="br-ui-feed">
          {[...entries].reverse().map((entry) => (
            <ConsoleEntryRow key={entry.seq} entry={entry} />
          ))}
        </ul>
      )}
    </div>
  )
}
