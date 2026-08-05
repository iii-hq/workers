/**
 * The Workers page (`#/ext/fleet`): who is connected, what each one brought
 * with it, and how it is holding up.
 *
 * Live on `engine::workers-available` (connect/disconnect) and
 * `engine::functions-available` (a worker's surface changing under it), so a
 * worker that dies goes grey here without a refresh and one that reconnects
 * flashes back in.
 *
 * The detail pane answers the question the console has been splitting across
 * three pages: this worker's functions, the trigger types it publishes, and
 * the live bindings pointing into it, all from one `engine::workers::info`
 * call, with its reported process metrics on top.
 */

import { EmptyState, type Host } from '@iii-dev/console-ui'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  listWorkers,
  useLiveSignals,
  useResource,
  type WorkerRow,
  workerInfo,
} from './engine'
import { summarize } from './trigger-kinds'
import {
  CatalogHead,
  CatalogRow,
  CatalogShell,
  Chip,
  CopyButton,
  DetailHead,
  ErrorNote,
  GroupHeader,
  LiveDot,
  Note,
  StatTile,
  useGroupToggle,
} from './widgets'

const DEAD = new Set(['disconnected', 'stopped', 'failed', 'error'])

function uptime(sinceMs: number, now: number): string {
  if (!sinceMs) return 'unknown'
  const seconds = Math.max(0, Math.round((now - sinceMs) / 1000))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  return hours < 48
    ? `${hours}h ${minutes % 60}m`
    : `${Math.floor(hours / 24)}d`
}

function bytes(value: unknown): string | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null
  const mb = value / (1024 * 1024)
  return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`
}

export function WorkersPage({ host }: { host: Host }) {
  const [search, setSearch] = useState('')
  const [selected, setSelected] = useState<string | null>(null)
  const groupState = useGroupToggle(() => true)

  const load = useCallback(() => listWorkers(host), [host])
  const workers = useResource(load)
  useLiveSignals(
    host,
    ['engine::workers-available', 'engine::functions-available'],
    workers.reload,
  )

  // Workers that connected on the last tick flash, the same signal the
  // functions list uses for new registrations.
  const [arrived, setArrived] = useState<ReadonlySet<string>>(new Set())
  const seenRef = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!workers.data) return
    const ids = new Set(workers.data.map((w) => w.id))
    const previous = seenRef.current
    seenRef.current = ids
    if (!previous) return
    const fresh = new Set([...ids].filter((id) => !previous.has(id)))
    if (fresh.size === 0) return
    setArrived(fresh)
    const timer = window.setTimeout(() => setArrived(new Set()), 2000)
    return () => window.clearTimeout(timer)
  }, [workers.data])

  const groups = useMemo(() => {
    const needle = search.trim().toLowerCase()
    const matched = (workers.data ?? []).filter(
      (w) =>
        !needle ||
        w.name.toLowerCase().includes(needle) ||
        (w.runtime ?? '').toLowerCase().includes(needle) ||
        w.status.toLowerCase().includes(needle),
    )
    const live = matched.filter((w) => !DEAD.has(w.status.toLowerCase()))
    const gone = matched.filter((w) => DEAD.has(w.status.toLowerCase()))
    const byName = (a: WorkerRow, b: WorkerRow) => a.name.localeCompare(b.name)
    return [
      { label: 'connected', items: live.sort(byName) },
      { label: 'gone', items: gone.sort(byName) },
    ].filter((g) => g.items.length > 0)
  }, [workers.data, search])

  const now = Date.now()
  const total = groups.reduce((n, g) => n + g.items.length, 0)
  const functionTotal = (workers.data ?? []).reduce(
    (n, w) => n + w.functionCount,
    0,
  )
  const busy = (workers.data ?? []).reduce((n, w) => n + w.activeInvocations, 0)

  return (
    <CatalogShell
      head={
        <CatalogHead
          title="fleet"
          count={`${total} workers · ${functionTotal} functions${busy ? ` · ${busy} running` : ''}`}
          search={search}
          onSearch={setSearch}
          searchPlaceholder="search workers, runtimes, status…"
          onRefresh={workers.reload}
          loading={workers.loading}
        >
          <LiveDot />
        </CatalogHead>
      }
      list={
        workers.error ? (
          <ErrorNote call="engine::workers::list" message={workers.error} />
        ) : workers.data === null ? (
          <Note>loading workers…</Note>
        ) : total === 0 ? (
          <EmptyState
            title={search.trim() ? 'nothing matches' : 'no workers connected'}
            description={
              search.trim()
                ? 'no worker name, runtime, or status contains that text.'
                : 'start a worker and it appears here as it connects.'
            }
          />
        ) : (
          groups.map((group) => (
            <div key={group.label} className="console-catalog-section">
              <GroupHeader
                label={group.label}
                meta={`${group.items.length}`}
                tone={group.label === 'connected' ? 'ok' : 'alert'}
                toneLabel={group.label === 'connected' ? 'live' : 'gone'}
                open={groupState.isOpen(group.label)}
                onToggle={() => groupState.toggle(group.label)}
              />
              {groupState.isOpen(group.label)
                ? group.items.map((worker) => (
                    <CatalogRow
                      key={worker.id}
                      primary={worker.name}
                      secondary={`${worker.runtime ?? 'unknown runtime'}${
                        worker.version ? ` ${worker.version}` : ''
                      } · ${worker.functionCount} functions · up ${uptime(
                        worker.connectedAtMs,
                        now,
                      )}${
                        worker.activeInvocations
                          ? ` · ${worker.activeInvocations} running`
                          : ''
                      }`}
                      selected={selected === worker.name}
                      flash={arrived.has(worker.id)}
                      onClick={() =>
                        setSelected((prev) =>
                          prev === worker.name ? null : worker.name,
                        )
                      }
                    />
                  ))
                : null}
            </div>
          ))
        )
      }
      detail={
        selected ? (
          <WorkerDetailPane
            host={host}
            name={selected}
            onClose={() => setSelected(null)}
          />
        ) : null
      }
    />
  )
}

function WorkerDetailPane({
  host,
  name,
  onClose,
}: {
  host: Host
  name: string
  onClose: () => void
}) {
  const load = useCallback(() => workerInfo(host, name), [host, name])
  const detail = useResource(load)
  useLiveSignals(
    host,
    ['engine::workers-available', 'engine::functions-available'],
    detail.reload,
  )

  if (detail.error) {
    return (
      <>
        <DetailHead title={name} onClose={onClose} />
        <ErrorNote call="engine::workers::info" message={detail.error} />
      </>
    )
  }
  if (!detail.data) {
    return (
      <>
        <DetailHead title={name} onClose={onClose} />
        <Note>loading worker…</Note>
      </>
    )
  }

  const { worker, metrics, functions, triggerTypes, bindings } = detail.data
  const heap = bytes(metrics?.memory_heap_used)
  const rss = bytes(metrics?.memory_rss)
  const cpu =
    typeof metrics?.cpu_percent === 'number'
      ? `${metrics.cpu_percent.toFixed(1)}%`
      : null
  const lag =
    typeof metrics?.event_loop_lag_ms === 'number'
      ? `${metrics.event_loop_lag_ms.toFixed(1)}ms`
      : null

  return (
    <>
      <DetailHead
        title={worker.name}
        subtitle={
          <>
            <Chip
              k="status"
              v={worker.status}
              tone={DEAD.has(worker.status.toLowerCase()) ? 'alert' : 'ok'}
            />
            {worker.runtime ? <Chip k="runtime" v={worker.runtime} /> : null}
            {worker.version ? <Chip k="version" v={worker.version} /> : null}
            {worker.tag ? <Chip k="tag" v={worker.tag} /> : null}
          </>
        }
        onClose={onClose}
      >
        <CopyButton value={worker.name} label="copy name" />
      </DetailHead>

      <div className="console-catalog-tabs">
        <div className="console-catalog-tiles">
          <StatTile
            label="uptime"
            value={uptime(worker.connectedAtMs, Date.now())}
          />
          <StatTile label="functions" value={String(worker.functionCount)} />
          <StatTile
            label="running now"
            value={String(worker.activeInvocations)}
            tone={worker.activeInvocations > 0 ? 'ok' : undefined}
          />
          {heap ? (
            <StatTile
              label="heap"
              value={heap}
              hint={rss ? `rss ${rss}` : undefined}
            />
          ) : null}
          {cpu ? <StatTile label="cpu" value={cpu} /> : null}
          {lag ? <StatTile label="event loop lag" value={lag} /> : null}
        </div>

        <Section title={`functions (${functions.length})`}>
          {functions.length === 0 ? (
            <Note>this worker registered no functions.</Note>
          ) : (
            functions.map((fn) => (
              <div key={fn.function_id} className="console-catalog-binding">
                <div className="console-catalog-binding-head">
                  <code>{fn.function_id}</code>
                  <CopyButton value={fn.function_id} />
                </div>
                {fn.description ? (
                  <span className="console-catalog-desc">{fn.description}</span>
                ) : null}
              </div>
            ))
          )}
        </Section>

        <Section title={`trigger types (${triggerTypes.length})`}>
          {triggerTypes.length === 0 ? (
            <Note>this worker publishes no trigger types.</Note>
          ) : (
            triggerTypes.map((type) => (
              <div key={type.id} className="console-catalog-binding">
                <div className="console-catalog-binding-head">
                  <code>{type.id}</code>
                </div>
                {type.description ? (
                  <span className="console-catalog-desc">
                    {type.description}
                  </span>
                ) : null}
              </div>
            ))
          )}
        </Section>

        <Section title={`bindings into this worker (${bindings.length})`}>
          {bindings.length === 0 ? (
            <Note>nothing is bound to this worker's functions.</Note>
          ) : (
            bindings.map((binding) => (
              <div key={binding.id} className="console-catalog-binding">
                <div className="console-catalog-binding-head">
                  <Chip k={binding.trigger_type} v={summarize(binding)} />
                </div>
                <code className="console-catalog-id">
                  {binding.function_id}
                </code>
              </div>
            ))
          )}
        </Section>
      </div>
    </>
  )
}

function Section({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <div className="console-catalog-worker-section">
      <span className="console-catalog-field-label">{title}</span>
      {children}
    </div>
  )
}
