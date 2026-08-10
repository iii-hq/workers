/**
 * Custom configuration form for the `sandbox-code-runner` configuration
 * entry — registered through `host.configForms`, replacing the console's
 * generic schema-driven form for this worker only. Mirrors the llm-router
 * form's contract: the form edits the working draft via `onChange`; dirty
 * tracking, save/reset, validation and the SaveBar stay host-owned.
 *
 * Three knobs (config.rs): default_timeout_ms (exec deadline when a
 * request carries none), max_timeout_ms (requests clamp to this),
 * idle_ttl_secs (sent to sandbox::create as idle_timeout_secs; the worker
 * floors it at 30s). Empty input = "use the default" — the key is deleted
 * from the entry, never written as 0.
 */

import { type ConfigFormProps, type JsonValue } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'
import { formatMs, formatSecs } from './format'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

interface Field {
  key: string
  label: string
  defaultValue: number
  echo: (n: number) => string
  hint?: string
}

const FIELDS: Field[] = [
  {
    key: 'default_timeout_ms',
    label: 'default timeout (ms)',
    defaultValue: 5_000,
    echo: formatMs,
    hint: 'used when a run/exec request carries no timeout_ms',
  },
  {
    key: 'max_timeout_ms',
    label: 'max timeout (ms)',
    defaultValue: 30_000,
    echo: formatMs,
    hint: 'requested timeouts clamp to this ceiling',
  },
  {
    key: 'idle_ttl_secs',
    label: 'idle ttl (secs)',
    defaultValue: 900,
    echo: formatSecs,
    hint: 'idle_timeout_secs on sandbox::create — the worker floors it at 30s',
  },
]

export function SandboxConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)

  const defaultTimeout = Number(value.default_timeout_ms ?? 5_000)
  const maxTimeout = Number(value.max_timeout_ms ?? 30_000)
  const defaultOverMax =
    Number.isFinite(defaultTimeout) &&
    Number.isFinite(maxTimeout) &&
    defaultTimeout > maxTimeout

  // Deep-link focus (#/workers/configuration/sandbox-code-runner/<field>):
  // the host only scroll-focuses the generic form's DOM ids, so honoring
  // the request is this override's job.
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const el = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    el?.scrollIntoView({ block: 'center' })
    el?.focus()
  }, [props.focusField])

  return (
    <div className="cr-page-cfg" ref={rootRef}>
      <span className="cr-page-cfg-caption">
        custom form · shipped by the sandbox-code-runner worker
      </span>

      <div className="cr-page-cfg-grid">
        {FIELDS.map((field) => {
          const set = typeof value[field.key] === 'number'
          const effective = set ? (value[field.key] as number) : field.defaultValue
          return (
            <div key={field.key}>
              <label className="cr-page-cfg-label" htmlFor={`cr-cfg-${field.key}`}>
                {field.label}
              </label>
              <input
                id={`cr-cfg-${field.key}`}
                data-field={field.key}
                className="cr-page-cfg-input"
                inputMode="numeric"
                value={set ? String(value[field.key]) : ''}
                placeholder={String(field.defaultValue)}
                onChange={(e) => {
                  const next = { ...value }
                  const n = Number(e.target.value)
                  if (e.target.value === '' || !Number.isFinite(n)) {
                    delete next[field.key]
                  } else {
                    next[field.key] = n
                  }
                  props.onChange(next)
                }}
              />
              <div className="cr-page-cfg-echo">
                {set
                  ? `= ${field.echo(effective)}`
                  : `default · ${field.echo(effective)}`}
              </div>
              {field.hint ? (
                <div className="cr-page-cfg-hint">{field.hint}</div>
              ) : null}
            </div>
          )
        })}
      </div>

      {defaultOverMax ? (
        <div className="cr-page-cfg-warning" role="alert">
          default timeout exceeds max — every default-timeout run will be
          clamped down to {formatMs(maxTimeout)}.
        </div>
      ) : null}

      {props.errors && props.errors.size > 0 ? (
        <div className="cr-page-cfg-errors">
          {[...props.errors].map(([path, message]) => (
            <div key={path}>
              <code>{path}</code> {message}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}
