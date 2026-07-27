/**
 * Custom configuration form for the `iii-directory` configuration entry —
 * registered through `host.configForms`, replacing the console's generic
 * schema-driven form for this worker only.
 *
 * The form edits the working draft via `onChange`; dirty tracking,
 * save/reset, and error mapping stay host-owned (the console's SaveBar
 * below the form drives `configuration::set`). Mirrors SkillsConfig
 * (workers/iii-directory/src/config.rs): skills_folder,
 * local_skills_folder, registry_url, download_timeout_ms,
 * registry_cache_ttl_ms, filter_unregistered, auto_download.
 */

import type { ConfigFormProps, JsonValue } from '@iii-dev/console-ui'
import { useEffect, useRef } from 'react'

type JsonObject = { [key: string]: JsonValue }

function asObject(v: JsonValue | undefined): JsonObject {
  return v && typeof v === 'object' && !Array.isArray(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

/** Fields from `SkillsConfig::topology()` — changing them needs a worker
 * restart (the hot-reload path refuses them). */
const TOPOLOGY_FIELDS = new Set([
  'skills_folder',
  'local_skills_folder',
  'auto_download',
])

export function DirectoryConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)

  const commit = (next: JsonObject) => props.onChange(next)

  const setString = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else next[field] = raw
    commit(next)
  }

  const setNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else if (!Number.isNaN(Number(raw))) next[field] = Number(raw)
    commit(next)
  }

  const setBool = (field: string, checked: boolean) => {
    commit({ ...value, [field]: checked })
  }

  // Deep-link focus: the host's own scroll+focus targets schema-form DOM
  // ids, so a custom form honors `focusField` itself.
  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field || !rootRef.current) return
    const target = rootRef.current.querySelector<HTMLElement>(
      `[data-field="${field}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [props.focusField])

  return (
    <div className="dir-ui-form" ref={rootRef}>
      <span className="dir-ui-form-caption">
        custom form · shipped by the iii-directory worker
      </span>

      <TextField
        field="skills_folder"
        label="skills folder"
        placeholder="~/.iii/skills"
        hint="global root every read scans and downloads write into — absolute, ~-prefixed, or CWD-relative"
        value={asString(value.skills_folder)}
        onChange={setString}
        errors={props.errors}
      />

      <TextField
        field="local_skills_folder"
        label="local skills folder"
        placeholder="./.iii/skills"
        hint="project-scoped overrides — a namespace directory here shadows the same namespace in the global folder entirely"
        value={asString(value.local_skills_folder)}
        onChange={setString}
        errors={props.errors}
      />

      <TextField
        field="registry_url"
        label="registry url"
        placeholder="https://api.workers.iii.dev"
        hint="workers registry the download + registry proxy functions call"
        value={asString(value.registry_url)}
        onChange={setString}
        errors={props.errors}
      />

      <NumberField
        field="download_timeout_ms"
        label="download timeout (ms)"
        placeholder="60000"
        hint="per download operation (HTTP request or git clone); also the registry proxy request timeout"
        value={value.download_timeout_ms}
        onChange={setNumber}
        errors={props.errors}
      />

      <NumberField
        field="registry_cache_ttl_ms"
        label="registry cache TTL (ms)"
        placeholder="60000"
        hint="how long registry list/info responses are cached in-process; 0 disables caching"
        value={value.registry_cache_ttl_ms}
        onChange={setNumber}
        errors={props.errors}
      />

      <CheckField
        field="filter_unregistered"
        label="hide skills of uninstalled workers"
        hint="reads only show namespaces matching a registered worker; off = everything on disk"
        checked={value.filter_unregistered !== false}
        onChange={setBool}
        errors={props.errors}
      />

      <CheckField
        field="auto_download"
        label="auto-download skills on worker add"
        hint="subscribes to worker add events and reconciles missing bundles at boot"
        checked={value.auto_download !== false}
        onChange={setBool}
        errors={props.errors}
      />

      {props.errors && props.errors.size > 0 ? (
        <div className="dir-ui-form-errors">
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

function FieldShell({
  field,
  label,
  hint,
  errors,
  children,
}: {
  field: string
  label: React.ReactNode
  hint?: string
  errors?: ConfigFormProps['errors']
  children: React.ReactNode
}) {
  const error = errors?.get(`/${field}`)
  return (
    <div className="dir-ui-field">
      <label htmlFor={`dir-cfg-${field}`}>
        {label}
        {TOPOLOGY_FIELDS.has(field) ? (
          <span className="dir-ui-restart-chip">restart required</span>
        ) : null}
      </label>
      {children}
      {error ? <span className="err">{error}</span> : null}
      {hint ? <span className="hint">{hint}</span> : null}
    </div>
  )
}

function TextField({
  field,
  label,
  placeholder,
  hint,
  value,
  onChange,
  errors,
}: {
  field: string
  label: string
  placeholder: string
  hint?: string
  value: string
  onChange: (field: string, raw: string) => void
  errors?: ConfigFormProps['errors']
}) {
  return (
    <FieldShell field={field} label={label} hint={hint} errors={errors}>
      <input
        id={`dir-cfg-${field}`}
        data-field={field}
        className="dir-ui-input"
        type="text"
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => onChange(field, e.target.value)}
      />
    </FieldShell>
  )
}

function NumberField({
  field,
  label,
  placeholder,
  hint,
  value,
  onChange,
  errors,
}: {
  field: string
  label: string
  placeholder: string
  hint?: string
  value: JsonValue | undefined
  onChange: (field: string, raw: string) => void
  errors?: ConfigFormProps['errors']
}) {
  return (
    <FieldShell field={field} label={label} hint={hint} errors={errors}>
      <input
        id={`dir-cfg-${field}`}
        data-field={field}
        className="dir-ui-input"
        type="number"
        min={0}
        value={typeof value === 'number' ? value : ''}
        placeholder={placeholder}
        onChange={(e) => onChange(field, e.target.value)}
      />
    </FieldShell>
  )
}

function CheckField({
  field,
  label,
  hint,
  checked,
  onChange,
  errors,
}: {
  field: string
  label: string
  hint?: string
  checked: boolean
  onChange: (field: string, checked: boolean) => void
  errors?: ConfigFormProps['errors']
}) {
  const error = errors?.get(`/${field}`)
  return (
    <div className="dir-ui-field">
      <span className="dir-ui-checkrow">
        <input
          id={`dir-cfg-${field}`}
          data-field={field}
          type="checkbox"
          checked={checked}
          onChange={(e) => onChange(field, e.target.checked)}
        />
        <label htmlFor={`dir-cfg-${field}`}>
          {label}
          {TOPOLOGY_FIELDS.has(field) ? (
            <span className="dir-ui-restart-chip">restart required</span>
          ) : null}
        </label>
      </span>
      {error ? <span className="err">{error}</span> : null}
      {hint ? <span className="hint">{hint}</span> : null}
    </div>
  )
}
