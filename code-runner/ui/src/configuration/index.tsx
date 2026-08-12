/**
 * Custom configuration form for the `code-runner` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * The form edits the working draft via `onChange`; dirty tracking,
 * save/reset, and error mapping stay host-owned (the console's SaveBar
 * below the form drives `configuration::set`). Mirrors CodeRunnerConfig
 * (workers/code-runner/src/config.rs) and its reload split: the output caps
 * and timeouts hot-apply on save, everything else is captured when the
 * engines boot and applies at the next worker restart — each section says
 * which it is, so an operator knows what to expect from the save.
 */

import { useEffect, useRef } from 'react'
import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'

type JsonObject = { [key: string]: JsonValue }

const DEFAULTS = {
  max_runtimes: 32,
  default_timeout_ms: 5000,
  max_timeout_ms: 30000,
  idle_ttl_secs: 900,
  heap_mb: 128,
  external_mb: 64,
  scratch_mb: 8,
  scratch_files: 64,
  max_result_bytes: 32768,
  max_stream_bytes: 16384,
} as const

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function numberOr(v: JsonValue | undefined, fallback: number): number {
  return typeof v === 'number' ? v : fallback
}

/** "32768 → ≈32 KiB (~8k tokens)"; the caps exist to stop context flooding,
 * so the token order of magnitude is the number an operator actually wants. */
function bytesHint(bytes: number): string {
  if (bytes === 0) return 'off — echoed uncapped'
  const kib = bytes / 1024
  const tokens = Math.round(bytes / 4 / 100) * 100
  const kibLabel = Number.isInteger(kib) ? String(kib) : kib.toFixed(1)
  return `≈${kibLabel} KiB (~${tokens >= 1000 ? `${Math.round(tokens / 100) / 10}k` : tokens} tokens)`
}

export function CodeRunnerConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)

  const setNumber = (field: keyof typeof DEFAULTS, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else {
      const n = Number(raw)
      if (!Number.isInteger(n) || n < 0) return
      next[field] = n
    }
    props.onChange(next)
  }

  const numberField = (
    field: keyof typeof DEFAULTS,
    label: string,
    hint?: string,
  ) => (
    <div className="cr-cfg-field">
      <label htmlFor={`cr-cfg-${field}`}>{label}</label>
      <input
        id={`cr-cfg-${field}`}
        data-field={field}
        className="cr-cfg-input"
        type="number"
        min={0}
        placeholder={String(DEFAULTS[field])}
        value={typeof value[field] === 'number' ? (value[field] as number) : ''}
        onChange={(e) => setNumber(field, e.target.value)}
      />
      {hint ? <span className="hint">{hint}</span> : null}
    </div>
  )

  // Deep-link focus (`#/workers/configuration/code-runner/<field>`): the
  // host's own scroll+focus targets schema-form DOM ids, so a custom form
  // honors `focusField` itself.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    // CSS.escape: the field name rides in on the URL fragment, and an
    // unescaped `"` or `]` makes querySelector throw during commit —
    // unmounting the whole form instead of skipping the focus.
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  const resultBytes = numberOr(value.max_result_bytes, DEFAULTS.max_result_bytes)
  const streamBytes = numberOr(value.max_stream_bytes, DEFAULTS.max_stream_bytes)
  const footprintMb =
    numberOr(value.max_runtimes, DEFAULTS.max_runtimes) *
    numberOr(value.scratch_mb, DEFAULTS.scratch_mb)

  return (
    <div className="cr-cfg-form" ref={rootRef}>
      <span className="cr-cfg-caption">
        custom form · shipped by the code-runner worker
      </span>

      <div className="cr-cfg-section">
        <span className="cr-cfg-section-title">
          output caps <span className="cr-cfg-live">hot-applies on save</span>
        </span>
        <span className="hint">
          Oversized run echoes are what flood a session's context: over the
          cap, `result` becomes an omission marker (write big values to
          iii.files instead) and stdout/stderr keep head+tail around a
          truncation marker. 0 turns a cap off.
        </span>
        {numberField('max_result_bytes', 'max result bytes', bytesHint(resultBytes))}
        {numberField('max_stream_bytes', 'max bytes per stream (stdout / stderr)', bytesHint(streamBytes))}
      </div>

      <div className="cr-cfg-section">
        <span className="cr-cfg-section-title">
          agent guidance <span className="cr-cfg-live">hot-applies on save</span>
        </span>
        <span className="cr-cfg-checkrow">
          <input
            id="cr-cfg-inject_guidance"
            data-field="inject_guidance"
            type="checkbox"
            checked={value.inject_guidance !== false}
            onChange={(e) =>
              props.onChange({ ...value, inject_guidance: e.target.checked })
            }
          />
          <label htmlFor="cr-cfg-inject_guidance">
            inject code-runner usage guidance into agent system prompts
          </label>
        </span>
        <span className="hint">
          The pre-generate hook that teaches agents this worker's surface
          (return conventions, keep/runtime_id, register_function). Off, the
          hook answers with a no-op and agents see only the function catalog.
        </span>
      </div>

      <div className="cr-cfg-section">
        <span className="cr-cfg-section-title">
          timeouts <span className="cr-cfg-live">hot-applies on save</span>
        </span>
        {numberField('default_timeout_ms', 'default timeout (ms, when a run omits timeout_ms)')}
        {numberField('max_timeout_ms', 'max timeout (ms, requests are clamped down to this)')}
      </div>

      <div className="cr-cfg-section">
        <span className="cr-cfg-section-title">
          runtimes &amp; memory{' '}
          <span className="cr-cfg-restart">applies at next worker restart</span>
        </span>
        {numberField('max_runtimes', 'max live runtimes (both engines)')}
        {numberField('idle_ttl_secs', 'idle TTL (seconds before an unused runtime is reaped)')}
        {numberField('heap_mb', 'V8 heap per node runtime (MiB)')}
        {numberField('external_mb', 'V8 off-heap per node runtime (MiB, ArrayBuffers)')}
      </div>

      <div className="cr-cfg-section">
        <span className="cr-cfg-section-title">
          scratch (node iii.files — python&apos;s /work budget is fixed by its engine){' '}
          <span className="cr-cfg-restart">applies at next worker restart</span>
        </span>
        {numberField(
          'scratch_mb',
          'scratch quota per node runtime (MiB, 0 removes the surface)',
          `worst-case host footprint: ${footprintMb} MiB — tmpfs (host RAM) on most Linux hosts unless scratch root points at real disk`,
        )}
        {numberField('scratch_files', 'max files per scratch directory')}
        <div className="cr-cfg-field">
          <label htmlFor="cr-cfg-scratch_root">scratch root (empty = system temp directory)</label>
          <input
            id="cr-cfg-scratch_root"
            data-field="scratch_root"
            className="cr-cfg-input"
            type="text"
            placeholder="/var/lib/iii/code-runner"
            value={typeof value.scratch_root === 'string' ? value.scratch_root : ''}
            onChange={(e) => {
              const next = { ...value }
              if (e.target.value === '') delete next.scratch_root
              else next.scratch_root = e.target.value
              props.onChange(next)
            }}
          />
        </div>
      </div>

      {props.errors && props.errors.size > 0 ? (
        <div className="cr-cfg-error">
          {[...props.errors.entries()].map(([pointer, message]) => (
            <div key={pointer}>
              {pointer ? `${pointer}: ` : ''}
              {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
