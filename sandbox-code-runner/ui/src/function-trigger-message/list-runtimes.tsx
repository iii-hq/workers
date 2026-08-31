/**
 * `sandbox-code-runner::list_runtimes` — the live rt-* registry: which
 * runtimes exist, what language each speaks, which sandbox microVM backs it,
 * and every bus function it registered. The request has no fields, so the
 * card is all response: one row per runtime, newest first (the worker's own
 * ordering), function ids rendered through the same redaction the other
 * cards use — a runtime id is a capability and never prints in full.
 */

import type {
  FunctionTriggerMessage,
  FunctionTriggerRenderer,
  Host,
} from '@iii-dev/console-ui'
import {
  asRecord,
  CardShell,
  DeniedCard,
  ErrorCard,
  deniedInfo,
  errorInfo,
  RegisteredIds,
  RuntimeChip,
  unwrapEnvelope,
} from '../lib/shared'

const FUNCTION_ID = 'sandbox-code-runner::list_runtimes'

interface RuntimeRow {
  runtime_id: string
  lang: string
  sandbox_id?: string
  created_at_ms?: number
  registered_functions: string[]
  vm_gone?: boolean
}

function parseRuntimes(output: unknown): RuntimeRow[] | null {
  const rec = asRecord(output)
  if (!rec || !Array.isArray(rec.runtimes)) return null
  const rows: RuntimeRow[] = []
  for (const entry of rec.runtimes) {
    const r = asRecord(entry)
    if (!r || typeof r.runtime_id !== 'string' || typeof r.lang !== 'string') {
      return null
    }
    rows.push({
      runtime_id: r.runtime_id,
      lang: r.lang,
      sandbox_id: typeof r.sandbox_id === 'string' ? r.sandbox_id : undefined,
      created_at_ms:
        typeof r.created_at_ms === 'number' && Number.isFinite(r.created_at_ms)
          ? r.created_at_ms
          : undefined,
      registered_functions: Array.isArray(r.registered_functions)
        ? r.registered_functions.filter((f): f is string => typeof f === 'string')
        : [],
      vm_gone: typeof r.vm_gone === 'boolean' ? r.vm_gone : undefined,
    })
  }
  return rows
}

function formatAge(createdAtMs: number | undefined): string | null {
  if (createdAtMs === undefined) return null
  const secs = Math.max(0, Math.floor((Date.now() - createdAtMs) / 1000))
  if (secs < 60) return `${secs}s`
  if (secs < 3600) return `${Math.floor(secs / 60)}m`
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`
}

function RuntimeRowView({ row }: { row: RuntimeRow }) {
  const age = formatAge(row.created_at_ms)
  return (
    <div className="cr-lsrt-row">
      <div className="cr-lsrt-head">
        <RuntimeChip runtimeId={row.runtime_id} />
        <span className="cr-ui-chip">{row.lang}</span>
        {row.sandbox_id ? (
          <span className="cr-ui-chip" title={row.sandbox_id}>
            <span className="k">vm </span>
            {row.sandbox_id.slice(0, 8)}…
          </span>
        ) : null}
        {age ? <span className="cr-ui-chip">{age}</span> : null}
        {row.vm_gone ? (
          <span
            className="cr-ui-chip cr-lsrt-gone"
            title="the backing VM left sandbox::list — idle-reaped or stopped"
          >
            vm gone
          </span>
        ) : null}
        <span className="cr-lsrt-count">
          {row.registered_functions.length === 0
            ? 'no bus functions'
            : `${row.registered_functions.length} bus function${row.registered_functions.length === 1 ? '' : 's'}`}
        </span>
      </div>
      <RegisteredIds ids={row.registered_functions} />
    </div>
  )
}

function SettledView({ message }: { message: FunctionTriggerMessage }) {
  const output = unwrapEnvelope(message.output)
  const denied = deniedInfo(output)
  if (denied) {
    return (
      <DeniedCard op="list_runtimes" reason={denied.reason} deniedBy={denied.deniedBy} />
    )
  }
  const error = errorInfo(output)
  if (error) return <ErrorCard op="list_runtimes" message={error.message} />
  const rows = parseRuntimes(output)
  if (rows === null) return null
  return (
    <CardShell op="list_runtimes">
      {rows.length === 0 ? (
        <div className="cr-ui-msg-note">
          · no live runtimes — the next `run keep=true` or `register_function`
          creates one
        </div>
      ) : (
        <div className="cr-lsrt-rows">
          {rows.map((row) => (
            <RuntimeRowView key={row.runtime_id} row={row} />
          ))}
        </div>
      )}
    </CardShell>
  )
}

export function createListRuntimesRenderer(_host: Host): FunctionTriggerRenderer {
  return {
    id: 'sandbox-code-runner/page.js#list-runtimes',
    isMatch: (functionId) => functionId === FUNCTION_ID,
    tryRender: (message) => {
      if (message.functionId !== FUNCTION_ID) return null
      if (message.running || message.pendingApproval) return null
      return <SettledView message={message} />
    },
    tryRenderRunning: (message) => {
      if (message.functionId !== FUNCTION_ID) return null
      return (
        <CardShell op="list_runtimes" running>
          <div className="cr-ui-msg-note">· listing live runtimes…</div>
        </CardShell>
      )
    },
  }
}
