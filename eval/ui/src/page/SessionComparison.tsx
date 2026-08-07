import { useCallback, useEffect, useMemo, useState, type CSSProperties } from 'react'
import { EmptyState, Skeleton, StatusPanel, type Host } from '@iii-dev/console-ui'
import { createEvalApi, errorMessage } from '../api'
import {
  formatDate,
  formatDelta,
  formatMetricValue,
  metricGroups,
  rootSessions,
  toggleSession,
  type ComparisonMetric,
} from '../sessionComparison'
import type {
  ContextSnapshot,
  SessionComparisonItem,
  SessionComparisonResponse,
  SessionMeta,
} from '../types'

function sessionLabel(session: SessionMeta): string {
  return session.title.trim() || session.session_id
}

function statusLabel(session: SessionMeta): string {
  return session.status === 'working' ? 'active' : session.status
}

function valueFor(item: SessionComparisonItem, key: string): number | null | undefined {
  return item.summary?.[key]
}

function metricValue(item: SessionComparisonItem, metric: ComparisonMetric): string {
  return formatMetricValue(valueFor(item, metric.key), metric.format)
}

function contextFor(item: SessionComparisonItem): ContextSnapshot | undefined {
  return item.metrics?.by_session.find(
    (session) => session.session_id === item.session.session_id,
  )?.context
}

function gridStyle(count: number): CSSProperties {
  return {
    gridTemplateColumns: `minmax(170px, 1.2fr) repeat(${count}, minmax(150px, 1fr))`,
  }
}

function ContextDetails({ item }: { item: SessionComparisonItem }) {
  const context = contextFor(item)
  if (!context) {
    return <span className="eval-ui-comparison-unavailable">—</span>
  }
  const categories = context.categories
  return (
    <div className="eval-ui-context-details">
      <div>
        <strong>{context.compacted ? 'latest snapshot · compacted' : 'latest snapshot'}</strong>
        <span>{formatDate(context.timestamp)}</span>
      </div>
      <div className="eval-ui-context-categories">
        <span>system {formatMetricValue(categories.system_prompt)}</span>
        <span>tools {formatMetricValue(categories.tools)}</span>
        <span>messages {formatMetricValue(categories.messages.user + categories.messages.assistant + categories.messages.function_result + categories.messages.custom)}</span>
        <span>overhead {formatMetricValue(categories.overhead)}</span>
        <span>hook {formatMetricValue(categories.hook_guidance)}</span>
      </div>
    </div>
  )
}

function LifecycleTable({ items }: { items: SessionComparisonItem[] }) {
  return (
    <div className="eval-ui-comparison-table eval-ui-comparison-lifecycle">
      <div className="eval-ui-comparison-row header" style={gridStyle(items.length)}>
        <span>lifecycle</span>
        {items.map((item) => (
          <span key={item.session.session_id}>{sessionLabel(item.session)}</span>
        ))}
      </div>
      {[
        ['session status', (item: SessionComparisonItem) => statusLabel(item.session)],
        ['turn status', (item: SessionComparisonItem) => item.lifecycle.turn_status ?? '—'],
        ['result error', (item: SessionComparisonItem) => item.lifecycle.result_error ?? '—'],
        ['complete', (item: SessionComparisonItem) => item.lifecycle.complete === undefined ? '—' : item.lifecycle.complete ? 'yes' : 'no'],
        ['partial snapshot', (item: SessionComparisonItem) => item.lifecycle.partial ? 'yes' : 'no'],
        ['terminal', (item: SessionComparisonItem) => item.lifecycle.terminal === undefined ? '—' : item.lifecycle.terminal ? 'yes' : 'no'],
        ['wake armed', (item: SessionComparisonItem) => item.lifecycle.expects_wake === undefined ? '—' : item.lifecycle.expects_wake ? 'yes' : 'no'],
      ].map(([label, read]) => (
        <div className="eval-ui-comparison-row" key={label as string} style={gridStyle(items.length)}>
          <span>{label as string}</span>
          {items.map((item) => <span key={item.session.session_id}>{(read as (item: SessionComparisonItem) => string)(item)}</span>)}
        </div>
      ))}
    </div>
  )
}

function ComparisonMatrix({ comparison }: { comparison: SessionComparisonResponse }) {
  const baseline = comparison.baseline_session_id
  return (
    <div className="eval-ui-comparison-result">
      <div className="eval-ui-comparison-capture">
        <span>live capture {formatDate(comparison.captured_at)}</span>
        <span>reference: {comparison.sessions.find((item) => item.session.session_id === baseline)?.session.title || baseline}</span>
      </div>
      <LifecycleTable items={comparison.sessions} />
      {metricGroups.map((group) => (
        <section className="eval-ui-comparison-group" key={group.label}>
          <h2>{group.label}</h2>
          <div className="eval-ui-comparison-table">
            <div className="eval-ui-comparison-row header" style={gridStyle(comparison.sessions.length)}>
              <span>metric</span>
              {comparison.sessions.map((item) => (
                <span key={item.session.session_id}>
                  {sessionLabel(item.session)}{item.session.session_id === baseline ? ' · reference' : ''}
                </span>
              ))}
            </div>
            {group.metrics.map((metric) => (
              <div className="eval-ui-comparison-row" key={metric.key} style={gridStyle(comparison.sessions.length)}>
                <span>{metric.label}</span>
                {comparison.sessions.map((item) => (
                  <span className="eval-ui-comparison-value" key={item.session.session_id}>
                    <strong>{metricValue(item, metric)}</strong>
                    {item.session.session_id === baseline ? null : (
                      <small>{formatDelta(item.deltas[metric.key], metric.format)}</small>
                    )}
                  </span>
                ))}
              </div>
            ))}
          </div>
          {group.label === 'Context' ? (
            <div className="eval-ui-context-grid">
              {comparison.sessions.map((item) => (
                <div className="eval-ui-context-card" key={item.session.session_id}>
                  <strong>{sessionLabel(item.session)}</strong>
                  <ContextDetails item={item} />
                </div>
              ))}
            </div>
          ) : null}
        </section>
      ))}
      {comparison.sessions.some((item) => item.errors.length > 0) ? (
        <div className="eval-ui-comparison-errors">
          {comparison.sessions.filter((item) => item.errors.length > 0).map((item) => (
            <div key={item.session.session_id}>
              <strong>{sessionLabel(item.session)}</strong>
              <span>{item.errors.join(' · ')}</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

export function SessionComparison({ host }: { host: Host }) {
  const api = useMemo(() => createEvalApi(host), [host])
  const [sessions, setSessions] = useState<SessionMeta[]>([])
  const [selected, setSelected] = useState<string[]>([])
  const [baseline, setBaseline] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [comparison, setComparison] = useState<SessionComparisonResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [comparing, setComparing] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const loadSessions = useCallback(async (quiet = false) => {
    if (quiet) setRefreshing(true)
    else setLoading(true)
    try {
      const next = rootSessions(await api.sessions())
      setSessions(next)
      setSelected((current) => current.filter((id) => next.some((session) => session.session_id === id)))
      setBaseline((current) => current && next.some((session) => session.session_id === current) ? current : null)
      setError(null)
    } catch (loadError) {
      setError(errorMessage(loadError))
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }, [api])

  useEffect(() => { void loadSessions() }, [loadSessions])

  const filteredSessions = sessions.filter((session) => {
    const needle = query.trim().toLowerCase()
    return !needle || `${sessionLabel(session)} ${session.session_id}`.toLowerCase().includes(needle)
  })

  const choose = (sessionId: string) => {
    setSelected((current) => {
      const next = toggleSession(current, sessionId)
      if (!next.includes(baseline ?? '')) setBaseline(next[0] ?? null)
      return next
    })
    setComparison(null)
  }

  const compare = async () => {
    if (selected.length < 2 || !baseline) return
    setComparing(true)
    try {
      setComparison(await api.compareSessions(selected, baseline))
      setError(null)
    } catch (compareError) {
      setError(errorMessage(compareError))
    } finally {
      setComparing(false)
    }
  }

  return (
    <div className="eval-ui-sessions">
      <div className="eval-ui-section-head">
        <div>
          <h1>Compare sessions</h1>
          <p>Choose 2–5 visible root sessions. You decide whether they are comparable; this view only shows current facts and deltas.</p>
        </div>
        <button className="eval-ui-button" type="button" onClick={() => void loadSessions(true)} disabled={refreshing}>
          {refreshing ? 'refreshing…' : 'refresh sessions'}
        </button>
      </div>
      <div className="eval-ui-comparison-rule">
        <strong>Live, descriptive comparison.</strong>
        <span>No snapshots, polling, score, ranking, or equivalence check are created.</span>
      </div>
      {error ? <StatusPanel variant="alert" headline="session comparison unavailable" detail={error} /> : null}
      {loading ? (
        <div className="eval-ui-loading"><Skeleton /><Skeleton /><Skeleton /></div>
      ) : sessions.length === 0 ? (
        <EmptyState title="no root sessions" description="Create or run a session in Console, then return here to compare its live metrics." />
      ) : (
        <>
          <section className="eval-ui-panel eval-ui-session-picker">
            <div className="eval-ui-panel-title">sessions · {selected.length}/5 selected</div>
            <input className="eval-ui-input" type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search title or session id" />
            <div className="eval-ui-session-list">
              {filteredSessions.map((session) => {
                const checked = selected.includes(session.session_id)
                return (
                  <div className={`eval-ui-session-option${checked ? ' selected' : ''}`} key={session.session_id}>
                    <label>
                      <input type="checkbox" checked={checked} onChange={() => choose(session.session_id)} />
                      <span>
                        <strong>{sessionLabel(session)}</strong>
                        <small>{statusLabel(session)} · {formatDate(session.updated_at)} · {session.session_id}</small>
                      </span>
                    </label>
                    <label className="eval-ui-baseline-choice">
                      <input type="radio" name="eval-baseline" checked={baseline === session.session_id} disabled={!checked} onChange={() => setBaseline(session.session_id)} />
                      reference
                    </label>
                  </div>
                )
              })}
            </div>
            <div className="eval-ui-comparison-actions">
              <span>{selected.length < 2 ? 'Select at least two sessions.' : baseline ? 'Reference is explicit and can be changed.' : 'Choose a reference session.'}</span>
              <button className="eval-ui-button primary" type="button" onClick={() => void compare()} disabled={selected.length < 2 || !baseline || comparing}>
                {comparing ? 'collecting…' : 'compare live metrics'}
              </button>
            </div>
          </section>
          {comparison ? <ComparisonMatrix comparison={comparison} /> : null}
        </>
      )}
    </div>
  )
}
