/**
 * Custom configuration form for the `database` configuration entry —
 * registered through `host.configForms` as the worker-owned editor.
 *
 * The overview lists query-history limits and connection resources. Opening
 * a connection pushes a dedicated deck level with its URL, driver-aware
 * capture, TLS, and pool settings. The form edits the working draft via
 * `onChange`; dirty tracking, save/reset, validation and the SaveBar stay
 * host-owned. Mirrors DatabaseConfig (database/src/config.rs).
 */

import {
  Button,
  Chip,
  type ConfigFormProps,
  type Host,
  Input,
  type JsonValue,
  List,
  ListItem,
  Panel,
  RawValueInput,
  Select,
  SettingsDeck,
  SettingsField,
  type SettingsFieldControlProps,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { errText } from '../lib/errors'
import {
  booleanLiteralForRawValue,
  DEFAULT_HISTORY_MAX_BYTES,
  DEFAULT_HISTORY_MAX_ENTRIES,
  type Driver,
  databaseFocusRequest,
  databaseHandleError,
  driverOfConfiguredUrl,
  isEnvironmentValue,
  isRawTypedValue,
  numberLiteralForRawValue,
  selectLiteralForRawValue,
  shouldMaskConfiguredUrl,
  shouldShowTlsForUrl,
} from './model'

type JsonObject = { [key: string]: JsonValue }

function isObjectValue(v: JsonValue | undefined): v is JsonObject {
  return v !== null && typeof v === 'object' && !Array.isArray(v)
}

function asObject(v: JsonValue | undefined): JsonObject {
  return isObjectValue(v) ? { ...v } : {}
}

function asString(v: JsonValue | undefined): string {
  return typeof v === 'string' ? v : ''
}

function asNumber(v: JsonValue | undefined, fallback: number): number {
  if (typeof v === 'number') return v
  return typeof v === 'string' ? numberLiteralForRawValue(v, fallback) : fallback
}

function asBoolean(v: JsonValue | undefined, fallback: boolean): boolean {
  if (typeof v === 'boolean') return v
  return typeof v === 'string' ? booleanLiteralForRawValue(v, fallback) : fallback
}

function pointer(parts: readonly string[]) {
  return `/${parts.map((part) => part.replaceAll('~', '~0').replaceAll('/', '~1')).join('/')}`
}

function fieldError(errors: ConfigFormProps['errors'], parts: readonly string[]) {
  const base = pointer(parts)
  return errors?.get(base) ?? [...(errors?.entries() ?? [])].find(([path]) => path.startsWith(`${base}/`))?.[1]
}

const CAPTURE_HINTS: Record<Driver, string> = {
  postgres:
    'Native: any client’s committed writes fire database::row-changed via triggers + LISTEN/NOTIFY. The role needs DDL rights on watched tables; bindings must name a table.',
  sqlite:
    'Native: triggers + changelog table + filesystem watch hear every process writing the file. Bindings must name a table.',
  mysql:
    'Native: streams the binlog as a replica — nothing installed in the schema, but the user needs GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.*',
  unknown: 'Set a URL first — capture support depends on the driver.',
}

const POOL_FIELDS = [
  {
    key: 'max',
    label: 'Maximum connections',
    description: 'Upper bound for open connections in this pool.',
    placeholder: '10',
  },
  {
    key: 'idle_timeout_ms',
    label: 'Idle timeout',
    description: 'Close unused connections after this many milliseconds.',
    placeholder: '30000',
  },
  {
    key: 'acquire_timeout_ms',
    label: 'Acquire timeout',
    description: 'Wait this many milliseconds for a connection before failing.',
    placeholder: '5000',
  },
] as const

const DRIVER_LABELS: Readonly<Record<Driver, string>> = {
  postgres: 'PostgreSQL',
  mysql: 'MySQL',
  sqlite: 'SQLite',
  unknown: 'Unknown',
}

/** Wire shape of `database::testConnection`. */
interface TestConnectionResp {
  ok: boolean
  driver: string
  latency_ms: number
  server_version?: string
  message?: string
}

type TestResult = { status: 'testing' } | { status: 'done'; ok: boolean; text: string }

function RawTypedValue({
  id,
  name,
  'data-field': dataField,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy,
  label,
  value,
  replacementLabel,
  onChange,
  onUseLiteral,
}: SettingsFieldControlProps & {
  label: string
  value: string
  replacementLabel: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  const environmentBacked = isEnvironmentValue(value) || value.trim().startsWith('${')
  return (
    <RawValueInput
      id={id}
      name={name}
      data-field={dataField}
      aria-invalid={ariaInvalid}
      aria-describedby={ariaDescribedBy}
      inputClassName="db-cfg-technical"
      type="text"
      label={label}
      kind={environmentBacked ? 'environment' : 'custom'}
      value={value}
      replacementLabel={replacementLabel}
      onChange={onChange}
      onUseLiteral={onUseLiteral}
    />
  )
}

function OpaqueSetting({
  id,
  field,
  label,
  description,
  value,
  error,
  replacementLabel,
  onChange,
  onUseLiteral,
}: {
  id: string
  field: string
  label: string
  description: string
  value: JsonValue
  error?: ReactNode
  replacementLabel: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  if (typeof value === 'string') {
    return (
      <SettingsField
        id={id}
        field={field}
        label={label}
        description={description}
        error={error}
        layout="stacked"
        controlSize="full"
        renderControl={(controlProps) => (
          <RawTypedValue
            {...controlProps}
            label={label}
            value={value}
            replacementLabel={replacementLabel}
            onChange={onChange}
            onUseLiteral={onUseLiteral}
          />
        )}
      />
    )
  }

  return (
    <SettingsField
      id={id}
      field={field}
      label={label}
      description={description}
      error={error}
      controlSize="fit"
      renderControl={(controlProps) => (
        <div className="db-cfg-opaque-action">
          <Chip tone="warning">Custom value preserved</Chip>
          <Button {...controlProps} type="button" variant="ghost" size="sm" onClick={onUseLiteral}>
            Use {replacementLabel}
          </Button>
        </div>
      )}
    />
  )
}

function DatabaseNumberInput({
  id,
  name,
  'data-field': dataField,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy,
  label,
  value,
  fallback,
  className = 'db-cfg-technical',
  onChange,
}: SettingsFieldControlProps & {
  label: string
  value: JsonValue | undefined
  fallback: number
  className?: string
  onChange(raw: string): void
}) {
  if (isRawTypedValue(value)) {
    const replacement = numberLiteralForRawValue(value, fallback)
    return (
      <RawTypedValue
        id={id}
        name={name}
        data-field={dataField}
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
        label={label}
        value={value}
        replacementLabel={String(replacement)}
        onChange={onChange}
        onUseLiteral={() => onChange(String(replacement))}
      />
    )
  }
  return (
    <Input
      id={id}
      name={name}
      data-field={dataField}
      className={className}
      type="number"
      min={0}
      inputMode="numeric"
      value={typeof value === 'number' ? String(value) : ''}
      placeholder={String(fallback)}
      aria-label={label}
      aria-invalid={ariaInvalid}
      aria-describedby={ariaDescribedBy}
      onChange={onChange}
    />
  )
}

function DatabaseSelectInput({
  id,
  name,
  'data-field': dataField,
  'aria-invalid': ariaInvalid,
  'aria-describedby': ariaDescribedBy,
  label,
  value,
  fallback,
  options,
  onChange,
}: SettingsFieldControlProps & {
  label: string
  value: JsonValue | undefined
  fallback: string
  options: readonly { value: string; label: string }[]
  onChange(next: string): void
}) {
  const current = typeof value === 'string' ? value : fallback
  const knownValue = options.some((option) => option.value === current)
  if (!knownValue) {
    const replacement = selectLiteralForRawValue(
      current,
      options.map((option) => option.value),
      fallback,
    )
    const replacementLabel = options.find((option) => option.value === replacement)?.label ?? replacement
    return (
      <RawTypedValue
        id={id}
        name={name}
        data-field={dataField}
        aria-invalid={ariaInvalid}
        aria-describedby={ariaDescribedBy}
        label={label}
        value={current}
        replacementLabel={replacementLabel}
        onChange={onChange}
        onUseLiteral={() => onChange(replacement)}
      />
    )
  }
  return (
    <Select
      id={id}
      name={name}
      data-field={dataField}
      className="db-cfg-control"
      value={current}
      options={[...options]}
      aria-label={label}
      aria-invalid={ariaInvalid}
      aria-describedby={ariaDescribedBy}
      sheetTitle={label}
      onChange={onChange}
    />
  )
}

function HistorySettings({
  value,
  errors,
  onChange,
}: {
  value: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (value: JsonObject) => void
}) {
  const entriesError = fieldError(errors, ['history_max_entries'])
  const bytesError = fieldError(errors, ['history_max_bytes'])
  const update = (field: 'history_max_entries' | 'history_max_bytes', raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else {
      const parsed = Number(raw)
      next[field] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
    }
    onChange(next)
  }

  return (
    <SettingsSection
      className="db-cfg-history"
      title="Query history"
      description="Per-database history is stored by the state worker. The first limit reached removes the oldest entries."
    >
      <SettingsList>
        <SettingsField
          id="db-cfg-history-max-entries"
          field="history_max_entries"
          label="Maximum entries"
          description="Set 0 to disable query history recording."
          meta={`${asNumber(value.history_max_entries, DEFAULT_HISTORY_MAX_ENTRIES).toLocaleString('en-US')} entries per database`}
          error={entriesError}
          controlSize="compact"
          renderControl={(controlProps) => (
            <DatabaseNumberInput
              {...controlProps}
              label="Maximum query history entries"
              value={value.history_max_entries}
              fallback={DEFAULT_HISTORY_MAX_ENTRIES}
              className="db-cfg-history-input db-cfg-technical"
              onChange={(raw) => update('history_max_entries', raw)}
            />
          )}
        />
        <SettingsField
          id="db-cfg-history-max-bytes"
          field="history_max_bytes"
          label="Maximum storage"
          description="Maximum JSON-serialized history size. Set 0 to disable recording."
          meta={`${asNumber(value.history_max_bytes, DEFAULT_HISTORY_MAX_BYTES).toLocaleString('en-US')} bytes per database`}
          error={bytesError}
          controlSize="compact"
          renderControl={(controlProps) => (
            <DatabaseNumberInput
              {...controlProps}
              label="Maximum query history storage in bytes"
              value={value.history_max_bytes}
              fallback={DEFAULT_HISTORY_MAX_BYTES}
              className="db-cfg-history-input db-cfg-technical"
              onChange={(raw) => update('history_max_bytes', raw)}
            />
          )}
        />
      </SettingsList>
    </SettingsSection>
  )
}

type DatabaseConfigFormProps = ConfigFormProps & { host: Host }

export function DatabaseConfigForm(props: DatabaseConfigFormProps) {
  if (!isObjectValue(props.value)) {
    const rootError = props.errors?.get('') ?? props.errors?.get('/')
    return (
      <div className="db-cfg">
        <SettingsSection
          title="Database configuration"
          description="This worker configuration is provided as one opaque value. It remains unchanged until you edit it or explicitly replace it."
        >
          <SettingsList>
            <OpaqueSetting
              id="db-cfg-root"
              field="configuration"
              label="Configuration value"
              description="Edit the template as-is, or replace it with a local SQLite connection."
              value={props.value}
              error={rootError}
              replacementLabel="SQLite defaults"
              onChange={props.onChange}
              onUseLiteral={() =>
                props.onChange({
                  databases: {
                    primary: { url: 'sqlite:./data/primary.db' },
                  },
                })
              }
            />
          </SettingsList>
        </SettingsSection>
      </div>
    )
  }

  return <StructuredDatabaseConfigForm {...props} rootValue={props.value} />
}

function StructuredDatabaseConfigForm(props: DatabaseConfigFormProps & { rootValue: JsonObject }) {
  const value = { ...props.rootValue }
  const databasesValue = value.databases
  const databasesOpaque = databasesValue !== undefined && !isObjectValue(databasesValue)
  const databases = asObject(databasesValue)
  const names = Object.keys(databases)
  const initialFocusRequest = databaseFocusRequest(names, props.focusField)
  const [selectedName, setSelectedName] = useState<string | null>(() => {
    const requested = initialFocusRequest?.databaseName
    return requested && Object.hasOwn(databases, requested) ? requested : null
  })
  const activeName = selectedName !== null && Object.hasOwn(databases, selectedName) ? selectedName : null

  // Probe outcomes are keyed by handle and dropped on any edit of that
  // handle — a stale "connected" next to a changed url would be a lie. The
  // token guards the async completion: an edit/rename/remove while a probe
  // is in flight bumps it, and the completion for the superseded probe is
  // discarded instead of resurrecting a result for a url it never tested.
  const [testResults, setTestResults] = useState<Record<string, TestResult>>({})
  const testTokens = useRef<Record<string, number>>(Object.create(null))
  const pendingHandleFocusRef = useRef('')

  const commit = (nextDatabases: JsonObject) => props.onChange({ ...value, databases: nextDatabases })

  const clearTest = (name: string) => {
    testTokens.current[name] = (testTokens.current[name] ?? 0) + 1
    setTestResults((r) => {
      const next = { ...r }
      delete next[name]
      return next
    })
  }

  const setDb = (name: string, next: JsonValue) => {
    clearTest(name)
    commit({ ...databases, [name]: next })
  }

  const runTest = async (name: string) => {
    const db = asObject(databases[name])
    const token = (testTokens.current[name] ?? 0) + 1
    testTokens.current[name] = token
    setTestResults((r) => ({ ...r, [name]: { status: 'testing' } }))
    let result: TestResult
    try {
      const resp = await props.host.iii.trigger<TestConnectionResp>(
        'database::testConnection',
        { url: asString(db.url), tls: db.tls ?? undefined, timeout_ms: 8000 },
        { timeoutMs: 10_000 },
      )
      result = {
        status: 'done',
        ok: resp.ok,
        text: resp.ok
          ? `Connected · ${resp.server_version ?? resp.driver} · ${resp.latency_ms}ms`
          : (resp.message ?? 'Connection failed'),
      }
    } catch (e) {
      result = { status: 'done', ok: false, text: errText(e) }
    }
    if (testTokens.current[name] !== token) return // superseded by an edit
    setTestResults((r) => ({ ...r, [name]: result }))
  }

  const removeDb = (name: string) => {
    clearTest(name)
    const next = { ...databases }
    delete next[name]
    commit(next)
  }

  const addDb = () => {
    let i = names.length + 1
    let name = names.length === 0 ? 'primary' : `db${i}`
    while (Object.hasOwn(databases, name)) name = `db${++i}`
    setSelectedName(name)
    commit({ ...databases, [name]: { url: `sqlite:./data/${name}.db` } })
  }

  const renameDb = (from: string, to: string) => {
    if (to === from || Object.hasOwn(databases, to)) return
    clearTest(from)
    // Rebuild in place so the card doesn't jump to the end of the list.
    const next = Object.fromEntries(names.map((key) => [key === from ? to : key, databases[key]])) as JsonObject
    pendingHandleFocusRef.current = to
    setSelectedName(to)
    commit(next)
  }

  // The host passes the deep-link path to the custom form. Prefer the exact
  // control, then fall back to the named database card.
  const rootRef = useRef<HTMLDivElement | null>(null)
  const handledFocusKeyRef = useRef('')
  const focusRequest = databaseFocusRequest(names, props.focusField)
  const focusRequestKey = focusRequest?.key ?? ''
  const focusExactField = focusRequest?.exactField ?? ''
  const focusDatabaseIndex = focusRequest?.databaseIndex ?? -1
  const focusDatabaseName = focusRequest?.databaseName ?? ''

  useEffect(() => {
    if (selectedName === null || Object.hasOwn(databases, selectedName)) return
    setSelectedName(null)
  }, [databases, selectedName])

  useEffect(() => {
    const pendingName = pendingHandleFocusRef.current
    if (!pendingName || activeName !== pendingName || !rootRef.current) return
    pendingHandleFocusRef.current = ''
    const frame = requestAnimationFrame(() => {
      rootRef.current
        ?.querySelector<HTMLElement>(`[data-field="${CSS.escape(`databases.${pendingName}.handle`)}"]`)
        ?.focus({ preventScroll: true })
    })
    return () => cancelAnimationFrame(frame)
  }, [activeName])

  useEffect(() => {
    if (!focusRequestKey) {
      handledFocusKeyRef.current = ''
      return
    }
    if (handledFocusKeyRef.current === focusRequestKey || !rootRef.current) return
    if (focusDatabaseIndex >= 0 && focusDatabaseName !== activeName) {
      setSelectedName(focusDatabaseName)
      return
    }
    if (focusDatabaseIndex < 0 && !focusDatabaseName && activeName !== null) {
      setSelectedName(null)
      return
    }

    const visiblePane =
      rootRef.current.querySelector<HTMLElement>('[data-settings-deck-pane]:not([hidden])') ?? rootRef.current
    const target =
      visiblePane.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusExactField)}"]`) ??
      (focusDatabaseName
        ? visiblePane.querySelector<HTMLElement>(`[data-field="db-${CSS.escape(focusDatabaseName)}"]`)
        : undefined)
    if (!target) return
    const focusTarget = target.matches('button, a[href], input, select, textarea, [tabindex]')
      ? target
      : target.querySelector<HTMLElement>('button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])')
    if (!focusTarget) return
    handledFocusKeyRef.current = focusRequestKey
    const frame = requestAnimationFrame(() => {
      focusTarget.focus({ preventScroll: true })
      focusTarget.scrollIntoView({ block: 'center' })
    })
    return () => cancelAnimationFrame(frame)
  }, [activeName, focusDatabaseIndex, focusDatabaseName, focusExactField, focusRequestKey])

  const overview = (
    <div className="db-cfg-overview">
      <HistorySettings value={value} errors={props.errors} onChange={props.onChange} />

      <SettingsSection
        className="db-cfg-connections"
        title="Connections"
        description="Each handle identifies a connection pool used by database functions."
        action={
          !databasesOpaque && names.length > 0 ? (
            <Button type="button" variant="ghost" size="sm" data-settings-deck-fallback onClick={addDb}>
              Add database
            </Button>
          ) : undefined
        }
      >
        {databasesOpaque ? (
          <SettingsList>
            <OpaqueSetting
              id="db-cfg-databases"
              field="databases"
              label="Connection collection"
              description="This configuration provides the complete database map as an opaque value. It remains unchanged until you edit it or explicitly replace it."
              value={databasesValue}
              error={fieldError(props.errors, ['databases'])}
              replacementLabel="an empty collection"
              onChange={(next) => props.onChange({ ...value, databases: next })}
              onUseLiteral={() => props.onChange({ ...value, databases: {} })}
            />
          </SettingsList>
        ) : names.length === 0 ? (
          <div className="db-cfg-empty">
            <strong>No databases configured</strong>
            <p>The worker needs at least one connection before it can start.</p>
            <Button type="button" size="sm" data-settings-deck-fallback data-settings-narrow-action onClick={addDb}>
              Add database
            </Button>
          </div>
        ) : (
          <Panel className="db-cfg-connection-panel">
            <List className="db-cfg-connection-list" role="group" aria-label="Configured databases">
              {names.map((name) => {
                const databaseValue = databases[name]
                const opaque = !isObjectValue(databaseValue)
                const db = asObject(databaseValue)
                const driver = driverOfConfiguredUrl(asString(db.url))
                const capture = asString(db.capture) || 'statements'
                const driverLabel =
                  driver === 'unknown' && asString(db.url).includes('${') ? 'From environment' : DRIVER_LABELS[driver]
                return (
                  <ListItem
                    key={name}
                    data-field={`db-${name}`}
                    aria-label={`Configure database ${name}`}
                    label={name}
                    description={
                      opaque
                        ? 'Custom configuration preserved'
                        : capture === 'native'
                          ? 'Native row capture'
                          : 'Worker statement capture'
                    }
                    trailing={
                      <span className="db-cfg-connection-meta">
                        <Chip tone={opaque || driver === 'unknown' ? 'warning' : 'accent'}>
                          {opaque ? 'Custom value' : driverLabel}
                        </Chip>
                        <span aria-hidden="true">›</span>
                      </span>
                    }
                    onClick={() => setSelectedName(name)}
                  />
                )
              })}
            </List>
          </Panel>
        )}
      </SettingsSection>
    </div>
  )

  return (
    <div className="db-cfg" ref={rootRef}>
      <SettingsDeck
        open={activeName !== null}
        title={activeName ?? 'Database connection'}
        description="Connection settings"
        backLabel="Connections"
        backAriaLabel="Back to database connections"
        autoFocusDetail={!focusRequestKey || handledFocusKeyRef.current === focusRequestKey}
        overview={overview}
        detail={
          activeName !== null ? (
            <DatabaseDetail
              key={activeName}
              name={activeName}
              names={names}
              value={databases[activeName]}
              onRename={(to) => renameDb(activeName, to)}
              onChange={(next) => setDb(activeName, next)}
              onRemove={() => {
                setSelectedName(null)
                removeDb(activeName)
              }}
              removable={names.length > 1}
              test={Object.hasOwn(testResults, activeName) ? testResults[activeName] : undefined}
              onTest={() => runTest(activeName)}
              errors={props.errors}
            />
          ) : null
        }
        onBack={() => setSelectedName(null)}
      />

      {props.errors && props.errors.size > 0 ? (
        <div className="db-cfg-errors">
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

interface DatabaseDetailProps {
  name: string
  names: readonly string[]
  value: JsonValue
  onRename: (to: string) => void
  onChange: (next: JsonValue) => void
  onRemove: () => void
  removable: boolean
  test: TestResult | undefined
  onTest: () => void
  errors: ConfigFormProps['errors']
}

function DatabaseDetail(card: DatabaseDetailProps) {
  if (isObjectValue(card.value)) {
    return <StructuredDatabaseDetail {...card} db={{ ...card.value }} />
  }

  const field = `databases.${card.name}`
  return (
    <div className="db-cfg-detail" data-field={`db-${card.name}`}>
      <SettingsSection
        className="db-cfg-detail-section"
        title="Connection payload"
        description="This connection is provided as an opaque value and will not be coerced by the Console."
        action={
          card.removable ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="db-cfg-danger-action"
              aria-label={`Remove database ${card.name}`}
              onClick={card.onRemove}
            >
              Remove
            </Button>
          ) : undefined
        }
      >
        <SettingsList>
          <OpaqueSetting
            id={`db-cfg-value-${card.name}`}
            field={field}
            label="Raw connection configuration"
            description="Edit the template as-is, or explicitly replace it with a regular SQLite connection."
            value={card.value}
            error={fieldError(card.errors, ['databases', card.name])}
            replacementLabel="SQLite defaults"
            onChange={card.onChange}
            onUseLiteral={() => card.onChange({ url: `sqlite:./data/${card.name}.db` })}
          />
        </SettingsList>
      </SettingsSection>
    </div>
  )
}

function StructuredDatabaseDetail(card: DatabaseDetailProps & { db: JsonObject }) {
  const { name, db } = card
  const url = asString(db.url)
  const driver = driverOfConfiguredUrl(url)
  const environmentUrl = url.includes('${')
  const showTls = shouldShowTlsForUrl(url)
  const maskUrl = shouldMaskConfiguredUrl(url)
  const captureValue = db.capture
  const capture = asString(captureValue) || 'statements'
  const tlsValue = db.tls
  const tlsOpaque = tlsValue !== undefined && !isObjectValue(tlsValue)
  const tls = asObject(tlsValue)
  const poolValue = db.pool
  const poolOpaque = poolValue !== undefined && !isObjectValue(poolValue)
  const pool = asObject(poolValue)
  const trustNativeRaw = isRawTypedValue(tls.trust_native) ? tls.trust_native : undefined
  const isMemorySqlite = driver === 'sqlite' && url.includes(':memory:')
  const [showUrl, setShowUrl] = useState(false)
  const [handleDraft, setHandleDraft] = useState(name)
  const [handleError, setHandleError] = useState<string | undefined>()
  const urlError = fieldError(card.errors, ['databases', name, 'url'])
  const captureError = fieldError(card.errors, ['databases', name, 'capture'])
  const tlsBlockError = fieldError(card.errors, ['databases', name, 'tls'])
  const poolBlockError = fieldError(card.errors, ['databases', name, 'pool'])
  const tlsModeError = fieldError(card.errors, ['databases', name, 'tls', 'mode'])
  const tlsCaError = fieldError(card.errors, ['databases', name, 'tls', 'ca_cert'])
  const tlsTrustError = fieldError(card.errors, ['databases', name, 'tls', 'trust_native'])
  const fieldId = (field: string) => `db-cfg-${field}-${name}`

  useEffect(() => {
    setHandleDraft(name)
    setHandleError(undefined)
    setShowUrl(false)
  }, [name])

  const set = (mutate: (next: JsonObject) => void) => {
    const next = { ...db }
    mutate(next)
    card.onChange(next)
  }

  const setBlock = (key: 'tls' | 'pool', mutate: (block: JsonObject) => void) =>
    set((next) => {
      const block = asObject(next[key])
      mutate(block)
      if (Object.keys(block).length > 0) next[key] = block
      else delete next[key]
    })

  const commitHandle = () => {
    const nextName = handleDraft.trim()
    const nextError = databaseHandleError(name, card.names, nextName)
    if (nextError) {
      setHandleError(nextError)
      return false
    }
    setHandleDraft(nextName)
    setHandleError(undefined)
    if (nextName !== name) card.onRename(nextName)
    return true
  }

  return (
    <div className="db-cfg-detail" data-field={`db-${name}`}>
      <SettingsSection
        className="db-cfg-detail-section"
        title="Connection"
        description="Connection identity, driver, and endpoint."
        action={
          card.removable ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="db-cfg-danger-action"
              aria-label={`Remove database ${name}`}
              onClick={card.onRemove}
            >
              Remove
            </Button>
          ) : undefined
        }
      >
        <SettingsList>
          <SettingsField
            id={fieldId('handle')}
            field={`databases.${name}.handle`}
            label="Handle"
            description="Used as the database argument in worker functions."
            error={handleError}
            controlSize="full"
            renderControl={(controlProps) => (
              <div className="db-cfg-handle-row">
                <Input
                  {...controlProps}
                  className="db-cfg-technical db-cfg-grow"
                  type="text"
                  value={handleDraft}
                  spellCheck={false}
                  autoComplete="off"
                  onChange={(next) => {
                    setHandleDraft(next)
                    setHandleError(databaseHandleError(name, card.names, next))
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault()
                      commitHandle()
                    } else if (event.key === 'Escape') {
                      event.preventDefault()
                      setHandleDraft(name)
                      setHandleError(undefined)
                    }
                  }}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={handleDraft.trim() === name || handleError !== undefined}
                  onClick={commitHandle}
                >
                  Rename
                </Button>
              </div>
            )}
          />
          <SettingsRow
            label="Driver"
            description="Inferred from the connection URL and not stored separately."
            control={
              <Chip tone={driver === 'unknown' ? 'warning' : 'accent'} data-field={`databases.${name}.driver`}>
                {driver === 'unknown' && environmentUrl ? 'From environment' : DRIVER_LABELS[driver]}
              </Chip>
            }
          />
          <SettingsField
            className="db-cfg-url-setting"
            layout="stacked"
            id={fieldId('url')}
            field={`databases.${name}.url`}
            label="Connection URL"
            description="PostgreSQL, MySQL, and SQLite URLs are supported. Credentials stay masked for network URLs."
            meta={
              card.test?.status === 'done' ? (
                <span className={card.test.ok ? 'db-cfg-test-ok' : 'db-cfg-test-fail'} role="status">
                  {card.test.text}
                </span>
              ) : undefined
            }
            error={urlError}
            controlSize="full"
            renderControl={(controlProps) => (
              <div className="db-cfg-url-row">
                <Input
                  {...controlProps}
                  className="db-cfg-technical db-cfg-grow"
                  type={!maskUrl || showUrl ? 'text' : 'password'}
                  value={url}
                  placeholder="postgres://user:pass@host:5432/db · mysql://… · sqlite:./data/app.db"
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(nextValue) => set((next) => (next.url = nextValue))}
                />
                {maskUrl ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    aria-label={showUrl ? 'Hide connection URL' : 'Show connection URL'}
                    aria-pressed={showUrl}
                    onClick={() => setShowUrl((current) => !current)}
                  >
                    {showUrl ? 'Hide' : 'Show'}
                  </Button>
                ) : null}
                <Button
                  type="button"
                  size="sm"
                  disabled={card.test?.status === 'testing' || url.trim() === ''}
                  onClick={card.onTest}
                >
                  {card.test?.status === 'testing' ? 'Testing…' : 'Test connection'}
                </Button>
              </div>
            )}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        className="db-cfg-detail-section"
        title="Change capture"
        description="Choose which writes can emit database::row-changed events."
      >
        <SettingsList>
          <SettingsField
            id={`db-cfg-capture-${name}`}
            field={`databases.${name}.capture`}
            label="Row-change capture"
            description={CAPTURE_HINTS[driver]}
            meta={
              capture === 'native' && isMemorySqlite ? (
                <span className="db-cfg-warn" role="alert">
                  A `:memory:` database is per-connection and cannot be captured.
                </span>
              ) : undefined
            }
            error={captureError}
            renderControl={(controlProps) => (
              <DatabaseSelectInput
                {...controlProps}
                label="Row-change capture"
                value={captureValue}
                fallback="statements"
                options={[
                  {
                    value: 'statements',
                    label: 'Statements — writes through this worker',
                  },
                  {
                    value: 'native',
                    label: 'Native — writes from any client',
                  },
                ]}
                onChange={(nextValue) =>
                  set((next) => {
                    if (nextValue === 'statements') delete next.capture
                    else next.capture = nextValue
                  })
                }
              />
            )}
          />
        </SettingsList>
      </SettingsSection>

      {showTls || tlsOpaque ? (
        <SettingsSection
          className="db-cfg-detail-section"
          title="Transport security"
          description="TLS settings remain available for network and environment-provided connection URLs."
        >
          <SettingsList>
            {tlsOpaque ? (
              <OpaqueSetting
                id={`db-cfg-tls-${name}`}
                field={`databases.${name}.tls`}
                label="TLS configuration"
                description="This connection supplies the entire TLS block as an opaque value."
                value={tlsValue}
                error={tlsBlockError}
                replacementLabel="TLS defaults"
                onChange={(raw) =>
                  set((next) => {
                    next.tls = raw
                  })
                }
                onUseLiteral={() =>
                  set((next) => {
                    delete next.tls
                  })
                }
              />
            ) : (
              <>
                <SettingsField
                  id={`db-cfg-tls-${name}`}
                  field={`databases.${name}.tls.mode`}
                  label="TLS mode"
                  description="Control encryption and hostname verification for network connections."
                  error={tlsModeError}
                  renderControl={(controlProps) => (
                    <DatabaseSelectInput
                      {...controlProps}
                      label="TLS mode"
                      value={tls.mode}
                      fallback="require"
                      options={[
                        { value: 'disable', label: 'Disable — plaintext' },
                        { value: 'require', label: 'Require — validate chain' },
                        {
                          value: 'verify-full',
                          label: 'Verify full — validate hostname',
                        },
                      ]}
                      onChange={(nextValue) =>
                        setBlock('tls', (block) => {
                          if (nextValue === 'require') delete block.mode
                          else block.mode = nextValue
                        })
                      }
                    />
                  )}
                />
                <SettingsField
                  id={`db-cfg-ca-${name}`}
                  field={`databases.${name}.tls.ca_cert`}
                  label="Extra CA bundle"
                  description="Optional path to a PEM bundle for private certificate authorities."
                  error={tlsCaError}
                  renderControl={(controlProps) => (
                    <Input
                      {...controlProps}
                      className="db-cfg-technical"
                      type="text"
                      value={asString(tls.ca_cert)}
                      placeholder="/etc/ssl/private-ca.pem"
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(nextValue) =>
                        setBlock('tls', (block) => {
                          if (nextValue === '') delete block.ca_cert
                          else block.ca_cert = nextValue
                        })
                      }
                    />
                  )}
                />
                <SettingsField
                  className="db-cfg-trust-row"
                  id={`db-cfg-trust-native-${name}`}
                  field={`databases.${name}.tls.trust_native`}
                  label="Use the system trust store"
                  description={
                    driver === 'mysql'
                      ? 'MySQL always includes its bundled public roots. This setting only changes PostgreSQL behavior.'
                      : 'Combine native system roots with the optional CA bundle. Turn off to trust only the supplied bundle.'
                  }
                  error={tlsTrustError}
                  layout={trustNativeRaw !== undefined ? 'stacked' : 'inline'}
                  controlSize={trustNativeRaw !== undefined ? 'full' : 'fit'}
                  renderControl={(controlProps) =>
                    trustNativeRaw !== undefined ? (
                      <RawTypedValue
                        {...controlProps}
                        label="Use the system trust store"
                        value={trustNativeRaw}
                        replacementLabel={booleanLiteralForRawValue(trustNativeRaw, true) ? 'on' : 'off'}
                        onChange={(raw) =>
                          setBlock('tls', (block) => {
                            block.trust_native = raw
                          })
                        }
                        onUseLiteral={() =>
                          setBlock('tls', (block) => {
                            if (booleanLiteralForRawValue(trustNativeRaw, true)) delete block.trust_native
                            else block.trust_native = false
                          })
                        }
                      />
                    ) : (
                      <Switch
                        {...controlProps}
                        aria-label="Use the system trust store"
                        checked={asBoolean(tls.trust_native, true)}
                        onChange={(event) =>
                          setBlock('tls', (block) => {
                            if (event.target.checked) delete block.trust_native
                            else block.trust_native = false
                          })
                        }
                      />
                    )
                  }
                />
              </>
            )}
          </SettingsList>
          {!tlsOpaque &&
          driver === 'postgres' &&
          !asBoolean(tls.trust_native, true) &&
          asString(tls.ca_cert).trim() === '' ? (
            <div className="db-cfg-notice" role="alert">
              Add a CA bundle before disabling the system trust store, or the PostgreSQL pool cannot establish trust.
            </div>
          ) : null}
        </SettingsSection>
      ) : null}

      <SettingsSection
        className="db-cfg-detail-section"
        title="Connection pool"
        description="Bound concurrency and how long callers wait for an available connection."
      >
        <SettingsList>
          {poolOpaque ? (
            <OpaqueSetting
              id={`db-cfg-pool-${name}`}
              field={`databases.${name}.pool`}
              label="Pool configuration"
              description="This connection supplies the entire pool block as an opaque value."
              value={poolValue}
              error={poolBlockError}
              replacementLabel="pool defaults"
              onChange={(raw) =>
                set((next) => {
                  next.pool = raw
                })
              }
              onUseLiteral={() =>
                set((next) => {
                  delete next.pool
                })
              }
            />
          ) : (
            POOL_FIELDS.map((field) => {
              const error = fieldError(card.errors, ['databases', name, 'pool', field.key])
              return (
                <SettingsField
                  key={field.key}
                  id={`db-cfg-${field.key}-${name}`}
                  field={`databases.${name}.pool.${field.key}`}
                  label={field.label}
                  description={field.description}
                  meta={`${Number(field.placeholder).toLocaleString('en-US')} by default`}
                  error={error}
                  controlSize="compact"
                  renderControl={(controlProps) => (
                    <DatabaseNumberInput
                      {...controlProps}
                      label={field.label}
                      value={pool[field.key]}
                      fallback={Number(field.placeholder)}
                      onChange={(raw) =>
                        setBlock('pool', (block) => {
                          if (raw.trim() === '') delete block[field.key]
                          else {
                            const parsed = Number(raw)
                            block[field.key] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
                          }
                        })
                      }
                    />
                  )}
                />
              )
            })
          )}
        </SettingsList>
      </SettingsSection>
    </div>
  )
}
