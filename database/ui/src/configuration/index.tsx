/**
 * Custom configuration form for the `database` configuration entry —
 * registered through `host.configForms` as the worker-owned editor.
 *
 * The overview lists query-history limits and connection resources. Opening
 * a connection pushes a dedicated deck level with its endpoint (a legacy URL
 * or the structured driver/host/port/user/database fields), display, TLS,
 * SSH tunnel, capture and pool settings. Credentials go to the secrets
 * worker and the entry keeps `secret://NAME`. The form edits the working
 * draft via `onChange`; dirty tracking, save/reset, validation and the
 * SaveBar stay host-owned. Mirrors DatabaseConfig (database/src/config.rs).
 */

import {
  Button,
  Chip,
  type ConfigFormProps,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  type Host,
  Input,
  type JsonValue,
  List,
  ListItem,
  Panel,
  RawValueInput,
  SegmentedControl,
  Select,
  SettingsDeck,
  SettingsField,
  type SettingsFieldControlProps,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { ConnectionIcon } from '../lib/connection-icons'
import { errText } from '../lib/errors'
import { type DbInfo, listDbs, testConnection } from '../lib/rpc'
import { storeSecret } from '../lib/secrets'
import {
  CONNECTION_DRIVERS,
  type ConnectionDriver,
  connectionDisplay,
  connectionDriver,
  connectionShape,
  connectionSummary,
  DEFAULT_PORTS,
  DEFAULT_SSH_PORT,
  isNetworkDriver,
  newConnectionEntry,
  parseConnectionUrl,
  type SensitiveField,
  secretNameFor,
  secretNamesInUse,
  supportsStructuredConnections,
  switchDriver,
  testConnectionPayload,
  testOutcomeText,
  uniqueSecretName,
  urlEntryToFields,
  valueKind,
} from './connection'
import { ColorPicker, IconPicker } from './ConnectionAppearance'
import {
  booleanLiteralForRawValue,
  DEFAULT_HISTORY_MAX_BYTES,
  DEFAULT_HISTORY_MAX_ENTRIES,
  type Driver,
  databaseFocusRequest,
  databaseHandleError,
  isEnvironmentValue,
  isRawTypedValue,
  LEGACY_TLS_MODES,
  numberLiteralForRawValue,
  selectLiteralForRawValue,
  shouldMaskConfiguredUrl,
  shouldShowTlsForUrl,
  TLS_MODES,
} from './model'
import { PemValueField } from './PemValueField'
import { SecretValueField } from './SecretValueField'

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
    'Native: any client’s committed writes fire database::row-changed via triggers + LISTEN/NOTIFY, carrying the changed rows’ primary keys (not their contents). The role needs DDL rights on watched tables; bindings must name a table.',
  sqlite:
    'Native: triggers + changelog table + filesystem watch hear every process writing the file, carrying the changed rows’ primary keys. Writers need libsqlite ≥ 3.38 (JSON built in). Bindings must name a table.',
  mysql:
    'Native: streams the binlog as a replica — nothing installed in the schema, but the user needs GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.*; primary keys ride the events only with binlog_row_metadata=FULL.',
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

type TestResult =
  | { status: 'testing' }
  | {
      status: 'done'
      ok: boolean
      text: string
      /** The SSH server's key, when the probe got that far — offered for pinning. */
      hostKey?: { algorithm: string; fingerprint: string }
    }

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
  options: readonly { value: string; label: string; description?: string }[]
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
  // Whether this worker takes structured entries (driver/host/…, ssh,
  // display, folder). An older worker keeps the url-only form.
  const structured = supportsStructuredConnections(props.schema)
  // Async completions (a secret stored, a url converted) must apply to the
  // draft as it is when they land, not as it was when they started.
  const valueRef = useRef(value)
  valueRef.current = value
  // What the running worker reports per connection — the `error` of a pool
  // it could not build (an unresolvable secret, say). Best-effort.
  const [live, setLive] = useState<Record<string, DbInfo>>({})
  useEffect(() => {
    let alive = true
    listDbs(props.host)
      .then((list) => {
        if (alive) setLive(Object.fromEntries(list.map((db) => [db.name, db])))
      })
      .catch(() => {})
    return () => {
      alive = false
    }
  }, [props.host])
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

  const commit = (nextDatabases: JsonObject) => props.onChange({ ...valueRef.current, databases: nextDatabases })
  const currentDatabases = () => asObject(valueRef.current.databases)

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
    commit({ ...currentDatabases(), [name]: next })
  }

  const runTest = async (name: string) => {
    const db = asObject(databases[name])
    const token = (testTokens.current[name] ?? 0) + 1
    testTokens.current[name] = token
    setTestResults((r) => ({ ...r, [name]: { status: 'testing' } }))
    let result: TestResult
    try {
      const resp = await testConnection(props.host, testConnectionPayload(name, db, structured))
      result = {
        status: 'done',
        ok: resp.ok,
        text: testOutcomeText(resp),
        hostKey: resp.ssh_host_key ?? undefined,
      }
    } catch (e) {
      result = { status: 'done', ok: false, text: errText(e) }
    }
    if (testTokens.current[name] !== token) return // superseded by an edit
    setTestResults((r) => ({ ...r, [name]: result }))
  }

  const removeDb = (name: string) => {
    clearTest(name)
    const next = { ...currentDatabases() }
    delete next[name]
    commit(next)
  }

  const addDb = (driver?: ConnectionDriver) => {
    let i = names.length + 1
    let name = names.length === 0 ? 'primary' : `db${i}`
    while (Object.hasOwn(databases, name)) name = `db${++i}`
    setSelectedName(name)
    commit({
      ...databases,
      [name]: driver ? newConnectionEntry(driver, name) : { url: `sqlite:./data/${name}.db` },
    })
  }

  // With structured entries, adding starts by picking the driver — the one
  // choice every other field depends on.
  const addAction = (attrs: Record<string, string>, variant?: 'ghost') =>
    structured ? (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" variant={variant} size="sm" {...attrs}>
            Add database
            <ChevronDown size={16} aria-hidden />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {CONNECTION_DRIVERS.map((d) => (
            <DropdownMenuItem key={d.value} onSelect={() => addDb(d.value)}>
              {d.label}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    ) : (
      <Button type="button" variant={variant} size="sm" {...attrs} onClick={() => addDb()}>
        Add database
      </Button>
    )

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
          !databasesOpaque && names.length > 0
            ? addAction({ 'data-settings-deck-fallback': 'true' }, 'ghost')
            : undefined
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
            {addAction({ 'data-settings-deck-fallback': 'true', 'data-settings-narrow-action': 'true' })}
          </div>
        ) : (
          <Panel className="db-cfg-connection-panel">
            <List className="db-cfg-connection-list" role="group" aria-label="Configured databases">
              {names.map((name) => {
                const databaseValue = databases[name]
                const opaque = !isObjectValue(databaseValue)
                const db = asObject(databaseValue)
                const driver = connectionDriver(db)
                const capture = asString(db.capture) || 'statements'
                const driverLabel =
                  driver === 'unknown' && asString(db.url).includes('${') ? 'From environment' : DRIVER_LABELS[driver]
                const display = connectionDisplay(db)
                const summary = connectionSummary(db)
                const failure = live[name]?.error
                return (
                  <ListItem
                    key={name}
                    data-field={`db-${name}`}
                    aria-label={`Configure database ${name}`}
                    leading={
                      <span className="db-cfg-conn-tile" data-tinted={display.color ? 'true' : undefined}>
                        <ConnectionIcon name={display.icon} color={display.color} />
                      </span>
                    }
                    label={name}
                    description={
                      opaque ? (
                        'Custom configuration preserved'
                      ) : failure ? (
                        <span className="db-cfg-warn">{failure}</span>
                      ) : (
                        [summary, capture === 'native' ? 'Native row capture' : 'Worker statement capture']
                          .filter(Boolean)
                          .join(' · ')
                      )
                    }
                    trailing={
                      <span className="db-cfg-connection-meta">
                        {failure ? <Chip tone="danger">Unavailable</Chip> : null}
                        <Chip tone={opaque || driver === 'unknown' ? 'warning' : 'accent'}>
                          {opaque ? 'Custom value' : driverLabel}
                        </Chip>
                        <ChevronRight size={16} aria-hidden />
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
              host={props.host}
              structured={structured}
              databases={databases}
              liveError={live[activeName]?.error}
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
  host: Host
  /** The worker takes structured entries (see `supportsStructuredConnections`). */
  structured: boolean
  /** The whole draft collection — for secret names other entries already use. */
  databases: JsonObject
  /** Why the running worker could not build this pool, if it could not. */
  liveError?: string
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
  const shape = connectionShape(db)
  const url = asString(db.url)
  const driver = connectionDriver(db)
  const network = isNetworkDriver(driver)
  const environmentUrl = url.includes('${')
  const showTls = shape === 'url' ? shouldShowTlsForUrl(url) : network
  const maskUrl = shape === 'url' && shouldMaskConfiguredUrl(url)
  const captureValue = db.capture
  const capture = asString(captureValue) || 'statements'
  const tlsValue = db.tls
  const tlsOpaque = tlsValue !== undefined && !isObjectValue(tlsValue)
  const tls = asObject(tlsValue)
  const sshValue = db.ssh
  const sshOpaque = sshValue !== undefined && !isObjectValue(sshValue)
  const ssh = asObject(sshValue)
  const sshOn = sshValue !== undefined
  const poolValue = db.pool
  const poolOpaque = poolValue !== undefined && !isObjectValue(poolValue)
  const pool = asObject(poolValue)
  const display = asObject(db.display)
  const trustNativeRaw = isRawTypedValue(tls.trust_native) ? tls.trust_native : undefined
  const isMemorySqlite = driver === 'sqlite' && (shape === 'url' ? url : asString(db.database)).includes(':memory:')
  const [showUrl, setShowUrl] = useState(false)
  const [handleDraft, setHandleDraft] = useState(name)
  const [handleError, setHandleError] = useState<string | undefined>()
  const [converting, setConverting] = useState(false)
  const [convertNote, setConvertNote] = useState<string | null>(null)
  // Network keys set aside by a switch to SQLite, restored on the way back.
  const driverStash = useRef<JsonObject>({})
  // The ssh block while the tunnel is switched off, restored when it is back on.
  const sshStash = useRef<JsonObject | null>(null)
  const latest = useRef(db)
  latest.current = db
  const errorAt = (...parts: string[]) => fieldError(card.errors, ['databases', name, ...parts])
  const urlError = errorAt('url')
  const captureError = errorAt('capture')
  const tlsBlockError = errorAt('tls')
  const sshBlockError = errorAt('ssh')
  const poolBlockError = errorAt('pool')
  const tlsModeError = errorAt('tls', 'mode')
  const tlsTrustError = errorAt('tls', 'trust_native')
  const fieldId = (field: string) => `db-cfg-${field}-${name}`
  const urlParse = shape === 'url' ? parseConnectionUrl(url) : null
  const test = card.test

  useEffect(() => {
    setHandleDraft(name)
    setHandleError(undefined)
    setShowUrl(false)
  }, [name])

  // Every edit starts from the entry as it is now: a secret stored or a url
  // converted after an await must not undo what was typed meanwhile.
  const set = (mutate: (next: JsonObject) => void) => {
    const next = { ...latest.current }
    mutate(next)
    latest.current = next
    card.onChange(next)
  }

  const setBlock = (key: 'tls' | 'pool' | 'ssh' | 'display', mutate: (block: JsonObject) => void) =>
    set((next) => {
      const block = asObject(next[key])
      mutate(block)
      if (Object.keys(block).length > 0 || key === 'ssh') next[key] = block
      else delete next[key]
    })

  /** Write one key (dotted for a block) or drop it when the value is empty. */
  const setField = (field: string, value: JsonValue | undefined) => {
    const [head, key] = field.split('.')
    const empty = value === undefined || value === ''
    if (key === undefined) {
      set((next) => {
        if (empty) delete next[head]
        else next[head] = value as JsonValue
      })
      return
    }
    setBlock(head as 'tls' | 'ssh' | 'display', (block) => {
      if (empty) delete block[key]
      else block[key] = value as JsonValue
    })
  }

  const secretName = (field: SensitiveField) =>
    uniqueSecretName(secretNameFor(name, field), secretNamesInUse(card.databases, { connection: name, field }))

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

  /** Legacy url → fields; a password the url carried goes to the secrets worker. */
  const convert = async () => {
    const entry = urlEntryToFields(latest.current)
    if (!entry) return
    setConverting(true)
    setConvertNote(null)
    try {
      if (valueKind(entry.password) === 'literal') {
        try {
          entry.password = await storeSecret(card.host, {
            name: secretName('password'),
            value: String(entry.password),
            description: `Password for database connection “${name}”, stored from the console`,
          })
        } catch (cause) {
          setConvertNote(`The password stayed in configuration: ${errText(cause)}`)
        }
      }
      latest.current = entry
      card.onChange(entry)
    } finally {
      setConverting(false)
    }
  }

  const testButton = (
    <Button
      type="button"
      size="sm"
      disabled={test?.status === 'testing' || (shape === 'url' && url.trim() === '')}
      onClick={card.onTest}
    >
      {test?.status === 'testing' ? 'Testing…' : 'Test connection'}
    </Button>
  )
  const testStatus =
    test?.status === 'done' ? (
      <span className={test.ok ? 'db-cfg-test-ok' : 'db-cfg-test-fail'} role="status">
        {test.text}
      </span>
    ) : undefined

  const textField = (
    field: string,
    label: string,
    description: string,
    placeholder: string,
    value: JsonValue | undefined,
  ) => (
    <SettingsField
      id={fieldId(field.replace('.', '-'))}
      field={`databases.${name}.${field}`}
      label={label}
      description={description}
      error={errorAt(...field.split('.'))}
      renderControl={(controlProps) =>
        typeof value === 'string' && isEnvironmentValue(value) ? (
          <RawTypedValue
            {...controlProps}
            label={label}
            value={value}
            replacementLabel="an empty value"
            onChange={(next) => setField(field, next)}
            onUseLiteral={() => setField(field, undefined)}
          />
        ) : (
          <Input
            {...controlProps}
            className="db-cfg-technical"
            type="text"
            value={asString(value)}
            placeholder={placeholder}
            spellCheck={false}
            autoComplete="off"
            onChange={(next) => setField(field, next)}
          />
        )
      }
    />
  )

  const portField = (field: 'port' | 'ssh.port', label: string, fallback: number, value: JsonValue | undefined) => (
    <SettingsField
      id={fieldId(field.replace('.', '-'))}
      field={`databases.${name}.${field}`}
      label={label}
      description={`${fallback} when empty.`}
      error={errorAt(...field.split('.'))}
      controlSize="compact"
      renderControl={(controlProps) => (
        <DatabaseNumberInput
          {...controlProps}
          label={label}
          value={value}
          fallback={fallback}
          onChange={(raw) => {
            if (raw.trim() === '') return setField(field, undefined)
            const parsed = Number(raw)
            setField(field, Number.isInteger(parsed) && parsed > 0 ? parsed : raw)
          }}
        />
      )}
    />
  )

  const secretField = (field: SensitiveField, label: string, description: string, multiline = false) => {
    const [head, key] = field.split('.')
    const value = key === undefined ? db[head] : asObject(db[head])[key]
    return (
      <SecretValueField
        host={card.host}
        id={fieldId(field.replace('.', '-'))}
        field={`databases.${name}.${field}`}
        label={label}
        description={description}
        error={errorAt(...field.split('.'))}
        value={value}
        secretName={secretName(field)}
        connection={name}
        multiline={multiline}
        onChange={(next) => setField(field, next)}
      />
    )
  }

  const pinned = asString(ssh.host_key)
  const presented = test?.status === 'done' ? test.hostKey : undefined
  const tlsModes = card.structured ? TLS_MODES : TLS_MODES.filter((m) => LEGACY_TLS_MODES.includes(m.value))

  return (
    <div className="db-cfg-detail" data-field={`db-${name}`}>
      {card.liveError ? (
        <div className="db-cfg-notice" role="alert">
          The worker could not open this connection: {card.liveError}
        </div>
      ) : null}
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
          {shape === 'url' ? (
            <>
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
                layout="stacked"
                id={fieldId('url')}
                field={`databases.${name}.url`}
                label="Connection URL"
                description="PostgreSQL, MySQL, and SQLite URLs are supported. Credentials stay masked for network URLs."
                meta={testStatus}
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
                    {testButton}
                  </div>
                )}
              />
              {card.structured ? (
                <SettingsRow
                  label="Connection fields"
                  description={
                    urlParse?.ok
                      ? 'Split this URL into driver, host, port, user, password and database fields. A password moves to the secrets worker.'
                      : `This URL stays a URL: ${urlParse && !urlParse.ok ? urlParse.reason : ''}`
                  }
                  meta={convertNote ? <span className="db-cfg-warn">{convertNote}</span> : undefined}
                  control={
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      data-field={`databases.${name}.convert`}
                      disabled={!urlParse?.ok || converting}
                      onClick={() => void convert()}
                    >
                      {converting ? 'Converting…' : 'Convert to fields'}
                    </Button>
                  }
                />
              ) : null}
            </>
          ) : (
            <>
              <SettingsField
                id={fieldId('driver')}
                field={`databases.${name}.driver`}
                label="Driver"
                description="The database server this connection talks to."
                error={errorAt('driver')}
                controlSize="full"
                renderControl={() => (
                  <SegmentedControl<ConnectionDriver>
                    variant="radio"
                    aria-label="Driver"
                    value={driver === 'unknown' ? 'postgres' : driver}
                    options={CONNECTION_DRIVERS.map((d) => ({ value: d.value, label: d.label, icon: false }))}
                    onChange={(next) => {
                      if (next === driver) return
                      const switched = switchDriver(latest.current, next, driverStash.current)
                      driverStash.current = switched.stash
                      latest.current = switched.entry
                      card.onChange(switched.entry)
                    }}
                  />
                )}
              />
              {network ? (
                <>
                  {textField('host', 'Host', 'Name or address of the database server.', 'db.example.com', db.host)}
                  {portField('port', 'Port', DEFAULT_PORTS[driver === 'mysql' ? 'mysql' : 'postgres'], db.port)}
                  {textField('user', 'User', 'The role or account to log in as.', 'app', db.user)}
                  {secretField('password', 'Password', 'Kept by the secrets worker; configuration holds a reference.')}
                  {textField('database', 'Database', 'The database to open on the server.', 'app', db.database)}
                </>
              ) : (
                textField(
                  'database',
                  'Database file',
                  'Path of the SQLite file on the engine host, or :memory: for a throwaway database.',
                  './data/app.db',
                  db.database,
                )
              )}
              <SettingsRow
                label="Test connection"
                description="Opens one throwaway connection with these settings — secrets resolved, through the tunnel when one is set."
                meta={testStatus}
                control={testButton}
              />
            </>
          )}
        </SettingsList>
      </SettingsSection>

      {card.structured ? (
        <SettingsSection
          className="db-cfg-detail-section"
          title="Display"
          description="How apps show this connection, and where its query files live."
        >
          <SettingsList>
            <SettingsField
              id={fieldId('icon')}
              field={`databases.${name}.display.icon`}
              label="Icon"
              layout="stacked"
              controlSize="full"
              error={errorAt('display', 'icon')}
              renderControl={() => (
                <IconPicker
                  label="Icon"
                  value={asString(display.icon) || undefined}
                  color={connectionDisplay(db).color}
                  onChange={(icon) => setField('display.icon', icon)}
                />
              )}
            />
            <SettingsField
              id={fieldId('color')}
              field={`databases.${name}.display.color`}
              label="Color"
              layout="stacked"
              controlSize="full"
              error={errorAt('display', 'color')}
              renderControl={() => (
                <ColorPicker
                  label="Color"
                  value={asString(display.color) || undefined}
                  onChange={(color) => setField('display.color', color)}
                />
              )}
            />
            {textField(
              'folder',
              'Queries folder',
              'Absolute path on the engine host that holds this connection’s .sql files. Optional.',
              '/home/me/project/queries',
              db.folder,
            )}
          </SettingsList>
        </SettingsSection>
      ) : null}

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
                  description={
                    tlsModes.find((m) => m.value === (asString(tls.mode) || 'require'))?.description ??
                    'Control encryption and hostname verification for network connections.'
                  }
                  error={tlsModeError}
                  renderControl={(controlProps) => (
                    <DatabaseSelectInput
                      {...controlProps}
                      label="TLS mode"
                      value={tls.mode}
                      fallback="require"
                      options={tlsModes.map((m) => ({ value: m.value, label: m.label, description: m.description }))}
                      onChange={(nextValue) =>
                        setBlock('tls', (block) => {
                          if (nextValue === 'require') delete block.mode
                          else block.mode = nextValue
                        })
                      }
                    />
                  )}
                />
                {card.structured ? (
                  <>
                    <PemValueField
                      id={`db-cfg-ca-${name}`}
                      field={`databases.${name}.tls.ca_cert`}
                      label="CA certificate"
                      description="Optional bundle for private certificate authorities: a path on the engine host, or paste / upload the PEM."
                      error={errorAt('tls', 'ca_cert')}
                      value={tls.ca_cert}
                      pathPlaceholder="/etc/ssl/private-ca.pem"
                      onChange={(next) => setField('tls.ca_cert', next)}
                    />
                    <PemValueField
                      id={`db-cfg-client-cert-${name}`}
                      field={`databases.${name}.tls.client_cert`}
                      label="Client certificate"
                      description="For servers that require mutual TLS. A path on the engine host, or paste / upload the PEM."
                      error={errorAt('tls', 'client_cert')}
                      value={tls.client_cert}
                      pathPlaceholder="/etc/ssl/client.crt"
                      onChange={(next) => setField('tls.client_cert', next)}
                    />
                    {secretField(
                      'tls.client_key',
                      'Client key',
                      'The client certificate’s private key, kept by the secrets worker.',
                      true,
                    )}
                  </>
                ) : (
                  <SettingsField
                    id={`db-cfg-ca-${name}`}
                    field={`databases.${name}.tls.ca_cert`}
                    label="Extra CA bundle"
                    description="Optional path to a PEM bundle for private certificate authorities."
                    error={errorAt('tls', 'ca_cert')}
                    renderControl={(controlProps) => (
                      <Input
                        {...controlProps}
                        className="db-cfg-technical"
                        type="text"
                        value={asString(tls.ca_cert)}
                        placeholder="/etc/ssl/private-ca.pem"
                        spellCheck={false}
                        autoComplete="off"
                        onChange={(nextValue) => setField('tls.ca_cert', nextValue)}
                      />
                    )}
                  />
                )}
                <SettingsField
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

      {card.structured && (network || sshOn) ? (
        <SettingsSection
          className="db-cfg-detail-section"
          title="SSH tunnel"
          description="Reach a database that only a bastion host can see. TLS still verifies the database host, not the tunnel."
        >
          <SettingsList>
            {sshOpaque ? (
              <OpaqueSetting
                id={`db-cfg-ssh-${name}`}
                field={`databases.${name}.ssh`}
                label="SSH configuration"
                description="This connection supplies the entire SSH block as an opaque value."
                value={sshValue}
                error={sshBlockError}
                replacementLabel="no tunnel"
                onChange={(raw) =>
                  set((next) => {
                    next.ssh = raw
                  })
                }
                onUseLiteral={() =>
                  set((next) => {
                    delete next.ssh
                  })
                }
              />
            ) : (
              <>
                <SettingsField
                  id={fieldId('ssh-enabled')}
                  field={`databases.${name}.ssh`}
                  label="Connect through an SSH tunnel"
                  description="The worker opens the tunnel and connects to the database host and port from the bastion."
                  error={sshBlockError}
                  controlSize="fit"
                  renderControl={(controlProps) => (
                    <Switch
                      {...controlProps}
                      aria-label="Connect through an SSH tunnel"
                      checked={sshOn}
                      onChange={(event) =>
                        set((next) => {
                          if (event.target.checked) {
                            next.ssh = sshStash.current ?? { host: '', user: '' }
                          } else {
                            sshStash.current = asObject(next.ssh)
                            delete next.ssh
                          }
                        })
                      }
                    />
                  )}
                />
                {sshOn ? (
                  <>
                    {textField('ssh.host', 'SSH host', 'The bastion’s name or address.', '203.0.113.7', ssh.host)}
                    {portField('ssh.port', 'SSH port', DEFAULT_SSH_PORT, ssh.port)}
                    {textField('ssh.user', 'SSH user', 'The account to log in to the bastion as.', 'ubuntu', ssh.user)}
                    {secretField('ssh.password', 'SSH password', 'Optional when a private key is set.')}
                    {secretField(
                      'ssh.private_key',
                      'Private key',
                      'OpenSSH or PEM (RSA, ECDSA, Ed25519): paste it or upload the file.',
                      true,
                    )}
                    {secretField('ssh.passphrase', 'Key passphrase', 'Only for an encrypted private key.')}
                    <SettingsField
                      id={fieldId('ssh-host-key')}
                      field={`databases.${name}.ssh.host_key`}
                      label="Pinned host key"
                      description="The bastion must present this key. Empty accepts whichever key it presents."
                      error={errorAt('ssh', 'host_key')}
                      layout="stacked"
                      controlSize="full"
                      meta={
                        presented ? (
                          pinned === presented.fingerprint ? (
                            <span className="db-cfg-test-ok">Matches the key the server presented</span>
                          ) : (
                            <span className="db-cfg-hostkey">
                              <span className="db-cfg-technical">
                                Server presented {presented.algorithm} {presented.fingerprint}
                              </span>
                              <Button
                                type="button"
                                size="sm"
                                onClick={() => setField('ssh.host_key', presented.fingerprint)}
                              >
                                Trust
                              </Button>
                            </span>
                          )
                        ) : undefined
                      }
                      renderControl={(controlProps) => (
                        <Input
                          {...controlProps}
                          className="db-cfg-technical"
                          type="text"
                          value={pinned}
                          placeholder="SHA256:… — run Test connection to see the server’s key"
                          spellCheck={false}
                          autoComplete="off"
                          onChange={(next) => setField('ssh.host_key', next)}
                        />
                      )}
                    />
                  </>
                ) : null}
              </>
            )}
          </SettingsList>
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
