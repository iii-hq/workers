/**
 * Purpose-built configuration editor for the browser worker. The Console
 * retains ownership of dirty tracking, validation, save, reset, and the
 * unsaved-change guard; this component only edits the JSON draft.
 */

import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  SettingsList,
  SettingsRow,
  SettingsSection,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { ChevronLeftIcon, GlobeIcon, useContainerNarrow } from '../lib/widgets'
import {
  booleanLiteralForRawValue,
  isEnvironmentValue,
  isRawTypedValue,
  numberLiteralForRawValue,
  selectLiteralForRawValue,
} from './template-values'

type JsonObject = { [key: string]: JsonValue }
export type SectionId = 'launch' | 'viewport' | 'limits' | 'behavior' | 'access' | 'scraping'
type FieldPath = string | readonly string[]
type OriginCapability = 'access' | 'downloads' | 'uploads' | 'scripting'
type OriginDecision = 'allow' | 'deny'

const CONFIG_NARROW_BELOW = 660
const LEGACY_WRAPPER = 'browser'
const BROWSER_OWNED_TOP_LEVEL_FIELDS = [
  'executable',
  'user_data_dir',
  'headless',
  'max_sessions',
  'console_buffer',
  'network_buffer',
  'viewport_width',
  'viewport_height',
  'default_timeout_ms',
  'max_timeout_ms',
  'idle_stop_ms',
  'screenshot_quality',
  'allowed_schemes',
  'max_snapshot_nodes',
  'default_origin_policy',
  'origin_policies',
  'allow_history_access',
  'allow_cookie_import',
  'allow_attach',
  'scrapling',
  // Legacy projection fields remain accepted by the Rust parser.
  'allow_loopback',
  'max_bulk_concurrency',
] as const
const DEFAULTS = {
  executable: '',
  user_data_dir: '',
  headless: true,
  max_sessions: 4,
  console_buffer: 500,
  network_buffer: 500,
  viewport_width: 1280,
  viewport_height: 800,
  default_timeout_ms: 30_000,
  max_timeout_ms: 120_000,
  idle_stop_ms: 300_000,
  screenshot_quality: 60,
  allowed_schemes: ['http', 'https', 'file'],
  max_snapshot_nodes: 2_000,
  allow_history_access: true,
  allow_cookie_import: true,
  allow_attach: false,
  scrapling: {
    security_mode: 'safe',
    chromium_executable: '',
    allow_loopback: false,
    defaults: {
      impersonate: 'chrome',
      headless: true,
      network_idle: false,
      proxy: '',
      include_html: false,
    },
    max_bulk_concurrency: 5,
    max_sessions: 8,
    session_idle_timeout_s: 900,
    adaptive_storage_path: './data/scrapling/elements.db',
    adaptive_max_bytes: 268_435_456,
    inject_guidance: true,
  },
} as const

const FIELD_SECTION: Record<string, SectionId> = {
  executable: 'launch',
  user_data_dir: 'launch',
  headless: 'launch',
  max_sessions: 'launch',
  allow_attach: 'launch',
  viewport_width: 'viewport',
  viewport_height: 'viewport',
  screenshot_quality: 'viewport',
  console_buffer: 'limits',
  network_buffer: 'limits',
  max_snapshot_nodes: 'limits',
  default_timeout_ms: 'behavior',
  max_timeout_ms: 'behavior',
  idle_stop_ms: 'behavior',
  allowed_schemes: 'behavior',
  default_origin_policy: 'access',
  origin_policies: 'access',
  allow_history_access: 'access',
  allow_cookie_import: 'access',
  scrapling: 'scraping',
}

const ORIGIN_CAPABILITIES: ReadonlyArray<{
  key: OriginCapability
  label: string
  description: string
}> = [
  {
    key: 'access',
    label: 'Page access',
    description: 'Open and read pages from this origin.',
  },
  {
    key: 'downloads',
    label: 'Downloads',
    description: 'Download files exposed by this origin.',
  },
  {
    key: 'uploads',
    label: 'Uploads',
    description: 'Send local files to this origin.',
  },
  {
    key: 'scripting',
    label: 'Scripting',
    description: 'Run page scripts and evaluations.',
  },
]

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value) ? { ...value } : {}
}

export function browserConfigurationValue(root: JsonValue): JsonObject {
  const object = asObject(root)
  if (!Object.hasOwn(object, LEGACY_WRAPPER)) return object
  return asObject(object[LEGACY_WRAPPER])
}

export function migrateBrowserConfiguration(root: JsonValue, nextValue: JsonObject): JsonObject {
  const object = asObject(root)
  if (!Object.hasOwn(object, LEGACY_WRAPPER)) return nextValue

  const migrated = { ...object }
  delete migrated[LEGACY_WRAPPER]
  for (const field of BROWSER_OWNED_TOP_LEVEL_FIELDS) delete migrated[field]
  return { ...migrated, ...nextValue }
}

function stringValue(value: JsonValue | undefined, fallback = ''): string {
  return typeof value === 'string' ? value : fallback
}

function numberValue(value: JsonValue | undefined, fallback: number): number {
  if (typeof value === 'number') return value
  return typeof value === 'string' ? numberLiteralForRawValue(value, fallback) : fallback
}

function booleanValue(value: JsonValue | undefined, fallback: boolean): boolean {
  if (typeof value === 'boolean') return value
  return typeof value === 'string' ? booleanLiteralForRawValue(value, fallback) : fallback
}

function pathParts(field: FieldPath): readonly string[] {
  return typeof field === 'string' ? [field] : field
}

function fieldName(field: FieldPath): string {
  return pathParts(field).join('.')
}

function pointer(field: FieldPath) {
  return `/${pathParts(field)
    .map((part) => part.replaceAll('~', '~0').replaceAll('/', '~1'))
    .join('/')}`
}

function fieldError(errors: ConfigFormProps['errors'], field: FieldPath) {
  const base = pointer(field)
  return errors?.get(base) ?? [...(errors?.entries() ?? [])].find(([path]) => path.startsWith(`${base}/`))?.[1]
}

function formatCount(value: number) {
  return new Intl.NumberFormat('en-US').format(value)
}

function formatDuration(ms: number) {
  if (ms === 0) return 'off'
  if (ms < 1_000) return `${ms}ms`
  const seconds = Math.round(ms / 1_000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  const remainder = seconds % 60
  return remainder ? `${minutes}m ${remainder}s` : `${minutes}m`
}

function formatBytes(bytes: number) {
  if (bytes < 1_024) return `${formatCount(bytes)} B`
  if (bytes < 1_048_576) return `${Math.round(bytes / 1_024)} KiB`
  return `${Math.round(bytes / 1_048_576)} MiB`
}

export function focusBrowserNarrowPane(
  root: HTMLElement | null,
  pane: 'nav' | 'editor',
  selection: SectionId,
): boolean {
  const target = root?.querySelector<HTMLElement>(
    pane === 'editor' ? '.br-cfg-editor' : `[data-config-section="${selection}"]`,
  )
  if (!target) return false
  target.focus({ preventScroll: true })
  target.scrollIntoView({ block: 'nearest' })
  return true
}

function Field({
  label,
  hint,
  error,
  children,
}: {
  label: ReactNode
  hint?: ReactNode
  error?: ReactNode
  children: ReactNode
}) {
  return (
    <SettingsRow
      className="br-cfg-field-row"
      layout="auto"
      label={label}
      description={hint}
      meta={
        error ? (
          <span className="br-cfg-error" role="alert">
            {error}
          </span>
        ) : undefined
      }
      control={<div className="br-cfg-row-control">{children}</div>}
    />
  )
}

function RawTypedValue({
  id,
  name,
  label,
  value,
  replacementLabel,
  error,
  errorId,
  onChange,
  onUseLiteral,
}: {
  id: string
  name: string
  label: string
  value: string
  replacementLabel: string
  error?: string
  errorId?: string
  onChange(next: string): void
  onUseLiteral(): void
}) {
  const environmentBacked = isEnvironmentValue(value) || /^\$\{[^}]*$/.test(value)
  return (
    <div className="br-cfg-template-control" data-environment-template={environmentBacked ? 'true' : 'false'}>
      <span className="br-cfg-template-kind">{environmentBacked ? 'Environment' : 'Custom value'}</span>
      <Input
        id={id}
        name={name}
        data-field={name}
        className="br-cfg-input br-cfg-template-input"
        type="text"
        value={value}
        preserveCase
        spellCheck={false}
        autoComplete="off"
        aria-label={`${label} raw value`}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        onChange={onChange}
      />
      <button
        type="button"
        className="br-cfg-template-replace"
        onClick={onUseLiteral}
        aria-label={`Replace ${label} environment value with ${replacementLabel}`}
      >
        Use {replacementLabel}
      </button>
    </div>
  )
}

function TextField({
  field,
  label,
  value,
  placeholder,
  hint,
  error,
  onChange,
}: {
  field: FieldPath
  label: string
  value: string
  placeholder?: string
  hint?: ReactNode
  error?: string
  onChange: (value: string) => void
}) {
  const name = fieldName(field)
  const id = `br-cfg-${name.replaceAll('.', '-')}`
  const errorId = error ? `${id}-error` : undefined
  return (
    <Field
      label={<label htmlFor={id}>{label}</label>}
      hint={hint}
      error={error ? <span id={errorId}>{error}</span> : undefined}
    >
      <Input
        id={id}
        name={name}
        data-field={name}
        className="br-cfg-input"
        value={value}
        preserveCase
        spellCheck={false}
        autoComplete="off"
        placeholder={placeholder}
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        onChange={onChange}
      />
    </Field>
  )
}

function NumberField({
  field,
  label,
  value,
  placeholder,
  min = 0,
  max,
  hint,
  error,
  onChange,
}: {
  field: FieldPath
  label: string
  value: JsonValue | undefined
  placeholder: number
  min?: number
  max?: number
  hint?: ReactNode
  error?: string
  onChange: (raw: string) => void
}) {
  const name = fieldName(field)
  const id = `br-cfg-${name.replaceAll('.', '-')}`
  const errorId = error ? `${id}-error` : undefined
  const replacement = isRawTypedValue(value) ? numberLiteralForRawValue(value, placeholder) : placeholder
  return (
    <Field
      label={<label htmlFor={id}>{label}</label>}
      hint={hint}
      error={error ? <span id={errorId}>{error}</span> : undefined}
    >
      {isRawTypedValue(value) ? (
        <RawTypedValue
          id={id}
          name={name}
          label={label}
          value={value}
          replacementLabel={String(replacement)}
          error={error}
          errorId={errorId}
          onChange={onChange}
          onUseLiteral={() => onChange(String(replacement))}
        />
      ) : (
        <Input
          id={id}
          name={name}
          data-field={name}
          className="br-cfg-input"
          type="number"
          min={min}
          max={max}
          inputMode="numeric"
          value={typeof value === 'number' ? String(value) : ''}
          preserveCase
          placeholder={String(placeholder)}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
          onChange={onChange}
        />
      )}
    </Field>
  )
}

function CheckField({
  field,
  label,
  hint,
  value,
  fallback,
  error,
  onChange,
}: {
  field: FieldPath
  label: string
  hint: ReactNode
  value: JsonValue | undefined
  fallback: boolean
  error?: string
  onChange: (value: boolean | string) => void
}) {
  const name = fieldName(field)
  const id = `br-cfg-${name.replaceAll('.', '-')}`
  const errorId = error ? `${id}-error` : undefined
  const replacement = isRawTypedValue(value) ? booleanLiteralForRawValue(value, fallback) : fallback
  return (
    <SettingsRow
      className="br-cfg-check-row"
      layout="auto"
      label={<label htmlFor={id}>{label}</label>}
      description={hint}
      control={
        isRawTypedValue(value) ? (
          <RawTypedValue
            id={id}
            name={name}
            label={label}
            value={value}
            replacementLabel={replacement ? 'on' : 'off'}
            error={error}
            errorId={errorId}
            onChange={onChange}
            onUseLiteral={() => onChange(replacement)}
          />
        ) : (
          <Switch
            id={id}
            name={name}
            data-field={name}
            aria-label={label}
            aria-invalid={Boolean(error)}
            aria-describedby={errorId}
            checked={typeof value === 'boolean' ? value : fallback}
            onChange={(event) => onChange(event.target.checked)}
          />
        )
      }
      meta={
        error ? (
          <span id={errorId} className="br-cfg-error" role="alert">
            {error}
          </span>
        ) : undefined
      }
    />
  )
}

function SchemesField({
  value,
  error,
  onChange,
}: {
  value: string[]
  error?: string
  onChange: (value: string[]) => void
}) {
  const canonical = value.join(', ')
  const errorId = error ? 'br-cfg-allowed_schemes-error' : undefined
  const [draft, setDraft] = useState(canonical)
  const lastEmitted = useRef(canonical)

  useEffect(() => {
    if (canonical === lastEmitted.current) return
    lastEmitted.current = canonical
    setDraft(canonical)
  }, [canonical])

  const update = (next: string) => {
    setDraft(next)
    const parsed = next
      .split(',')
      .map((scheme) => scheme.trim())
      .filter(Boolean)
    lastEmitted.current = parsed.join(', ')
    onChange(parsed)
  }

  return (
    <Field
      label={<label htmlFor="br-cfg-allowed_schemes">Allowed URL schemes</label>}
      hint="Enter a comma-separated list without ://. Keep this list as narrow as your workflows allow."
      error={error ? <span id={errorId}>{error}</span> : undefined}
    >
      <Input
        id="br-cfg-allowed_schemes"
        name="allowed_schemes"
        data-field="allowed_schemes"
        className="br-cfg-input"
        value={draft}
        preserveCase
        spellCheck={false}
        autoComplete="off"
        placeholder="http, https"
        aria-invalid={Boolean(error)}
        aria-describedby={errorId}
        onChange={update}
        onKeyDown={(event) => {
          if (event.key !== 'Enter') return
          event.preventDefault()
          event.currentTarget.blur()
        }}
      />
    </Field>
  )
}

function SelectField({
  field,
  label,
  value,
  options,
  hint,
  error,
  onChange,
}: {
  field: FieldPath
  label: string
  value: string
  options: ReadonlyArray<{ value: string; label: string }>
  hint?: ReactNode
  error?: string
  onChange: (value: string) => void
}) {
  const name = fieldName(field)
  const id = `br-cfg-${name.replaceAll('.', '-')}`
  const errorId = error ? `${id}-error` : undefined
  const knownValue = options.some((option) => option.value === value)
  const replacement = selectLiteralForRawValue(
    value,
    options.map((option) => option.value),
    options[0]?.value ?? '',
  )
  const replacementLabel = options.find((option) => option.value === replacement)?.label ?? replacement
  return (
    <Field
      label={<label htmlFor={id}>{label}</label>}
      hint={hint}
      error={error ? <span id={errorId}>{error}</span> : undefined}
    >
      {!knownValue ? (
        <RawTypedValue
          id={id}
          name={name}
          label={label}
          value={value}
          replacementLabel={replacementLabel}
          error={error}
          errorId={errorId}
          onChange={onChange}
          onUseLiteral={() => onChange(replacement)}
        />
      ) : (
        <select
          id={id}
          name={name}
          data-field={name}
          className="br-cfg-select"
          value={value}
          aria-invalid={Boolean(error)}
          aria-describedby={errorId}
          onChange={(event) => onChange(event.target.value)}
        >
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      )}
    </Field>
  )
}

function policyDecision(policy: JsonObject, capability: OriginCapability): '' | OriginDecision {
  const value = policy[capability]
  return value === 'allow' || value === 'deny' ? value : ''
}

function updatePolicy(policy: JsonObject, capability: OriginCapability, decision: string): JsonObject {
  const next = { ...policy }
  if (decision === 'allow' || decision === 'deny') {
    next[capability] = decision
  } else {
    delete next[capability]
  }
  return next
}

function PolicyControls({
  path,
  policy,
  errors,
  onChange,
}: {
  path: readonly string[]
  policy: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (policy: JsonObject) => void
}) {
  return (
    <SettingsList className="br-cfg-policy-grid">
      {ORIGIN_CAPABILITIES.map((capability) => (
        <SelectField
          key={capability.key}
          field={[...path, capability.key]}
          label={capability.label}
          value={policyDecision(policy, capability.key)}
          options={[
            { value: '', label: 'Use inherited policy' },
            { value: 'allow', label: 'Allow' },
            { value: 'deny', label: 'Deny' },
          ]}
          hint={capability.description}
          error={fieldError(errors, [...path, capability.key])}
          onChange={(decision) => onChange(updatePolicy(policy, capability.key, decision))}
        />
      ))}
    </SettingsList>
  )
}

function DefaultOriginPolicy({
  value,
  errors,
  onChange,
}: {
  value: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (value: JsonObject) => void
}) {
  const policy = asObject(value.default_origin_policy)
  const commit = (nextPolicy: JsonObject) => {
    const next = { ...value }
    if (Object.keys(nextPolicy).length === 0) delete next.default_origin_policy
    else next.default_origin_policy = nextPolicy
    onChange(next)
  }

  return (
    <div className="br-cfg-policy-card" data-field="default_origin_policy" tabIndex={-1}>
      <div className="br-cfg-policy-head">
        <div>
          <h5>Fallback policy</h5>
          <p>
            Used when an exact origin or bare hostname does not have its own entry. Unspecified capabilities remain
            allowed.
          </p>
        </div>
      </div>
      <PolicyControls path={['default_origin_policy']} policy={policy} errors={errors} onChange={commit} />
    </div>
  )
}

function OriginPolicies({
  value,
  errors,
  onChange,
}: {
  value: JsonObject
  errors: ConfigFormProps['errors']
  onChange: (value: JsonObject) => void
}) {
  const policies = asObject(value.origin_policies)
  const origins = Object.keys(policies)

  const commit = (nextPolicies: JsonObject) => {
    const next = { ...value }
    if (Object.keys(nextPolicies).length === 0) delete next.origin_policies
    else next.origin_policies = nextPolicies
    onChange(next)
  }

  const addOrigin = () => {
    let index = origins.length + 1
    let origin = origins.length === 0 ? 'example.com' : `origin-${index}.example`
    while (policies[origin] !== undefined) {
      origin = `origin-${++index}.example`
    }
    commit({ ...policies, [origin]: {} })
  }

  const renameOrigin = (from: string, raw: string) => {
    const to = raw
    if (to === from || policies[to] !== undefined) return
    const next: JsonObject = {}
    for (const origin of origins) {
      next[origin === from ? to : origin] = policies[origin]
    }
    commit(next)
  }

  return (
    <div className="br-cfg-origin-policies" data-field="origin_policies" tabIndex={-1}>
      <div className="br-cfg-origin-list">
        {origins.map((origin, index) => {
          const policy = asObject(policies[origin])
          const originError = fieldError(errors, ['origin_policies', origin])
          const originErrorId = originError ? `br-cfg-origin-${index}-error` : undefined
          return (
            // biome-ignore lint/suspicious/noArrayIndexKey: the stable row lets an origin key be renamed without remounting and losing input focus.
            <div className="br-cfg-policy-card" key={index}>
              <div className="br-cfg-policy-head">
                <div className="br-cfg-origin-name">
                  <label htmlFor={`br-cfg-origin-${index}`}>Origin or hostname</label>
                  <Input
                    id={`br-cfg-origin-${index}`}
                    name={`origin_policies.${origin}.origin`}
                    value={origin}
                    className="br-cfg-input"
                    preserveCase
                    spellCheck={false}
                    autoComplete="off"
                    placeholder="https://app.example.com or example.com"
                    aria-invalid={Boolean(originError)}
                    aria-describedby={originErrorId}
                    onChange={(next) => renameOrigin(origin, next)}
                    onBlur={(event) => renameOrigin(origin, event.target.value.trim())}
                  />
                </div>
                <button
                  type="button"
                  className="br-cfg-policy-remove"
                  aria-label={`Remove origin policy for ${origin || 'unnamed origin'}`}
                  onClick={() => {
                    const next = { ...policies }
                    delete next[origin]
                    commit(next)
                  }}
                >
                  Remove
                </button>
              </div>
              <PolicyControls
                path={['origin_policies', origin]}
                policy={policy}
                errors={errors}
                onChange={(nextPolicy) => commit({ ...policies, [origin]: nextPolicy })}
              />
              {originError ? (
                <p id={originErrorId} className="br-cfg-error" role="alert">
                  {originError}
                </p>
              ) : null}
            </div>
          )
        })}
      </div>
      <button type="button" className="br-cfg-policy-add" onClick={addOrigin}>
        Add origin policy
      </button>
    </div>
  )
}

function ConfigSection({ title, description, children }: { title: string; description: string; children: ReactNode }) {
  return (
    <SettingsSection className="br-cfg-section" title={title} description={description}>
      {children}
    </SettingsSection>
  )
}

function ConfigNav({
  value,
  selection,
  onSelect,
}: {
  value: JsonObject
  selection: SectionId
  onSelect: (section: SectionId) => void
}) {
  const width = numberValue(value.viewport_width, DEFAULTS.viewport_width)
  const height = numberValue(value.viewport_height, DEFAULTS.viewport_height)
  const maxSessions = numberValue(value.max_sessions, DEFAULTS.max_sessions)
  const headless = booleanValue(value.headless, DEFAULTS.headless)
  const consoleBuffer = numberValue(value.console_buffer, DEFAULTS.console_buffer)
  const networkBuffer = numberValue(value.network_buffer, DEFAULTS.network_buffer)
  const timeout = numberValue(value.default_timeout_ms, DEFAULTS.default_timeout_ms)
  const idle = numberValue(value.idle_stop_ms, DEFAULTS.idle_stop_ms)
  const originCount = Object.keys(asObject(value.origin_policies)).length
  const history = booleanValue(value.allow_history_access, DEFAULTS.allow_history_access)
  const cookies = booleanValue(value.allow_cookie_import, DEFAULTS.allow_cookie_import)
  const scrapling = asObject(value.scrapling)

  const sections: Array<{
    id: SectionId
    label: string
    description: string
    summary: string
  }> = [
    {
      id: 'launch',
      label: 'Launch',
      description: 'Process and sessions',
      summary: `${headless ? 'headless' : 'headful'} · ${maxSessions} max`,
    },
    {
      id: 'viewport',
      label: 'Viewport',
      description: 'Canvas and screenshots',
      summary: `${width} × ${height}`,
    },
    {
      id: 'limits',
      label: 'Capture limits',
      description: 'Buffers and snapshots',
      summary: `${formatCount(consoleBuffer)} / ${formatCount(networkBuffer)}`,
    },
    {
      id: 'behavior',
      label: 'Behavior',
      description: 'Timeouts and navigation',
      summary: `${formatDuration(timeout)} · idle ${formatDuration(idle)}`,
    },
    {
      id: 'access',
      label: 'Access policy',
      description: 'Origins and sensitive surfaces',
      summary: `${originCount} origin ${originCount === 1 ? 'rule' : 'rules'} · ${history && cookies ? 'surfaces on' : 'restricted'}`,
    },
    {
      id: 'scraping',
      label: 'Scraping',
      description: 'Runtime, fetch defaults, and guidance',
      summary: `${numberValue(scrapling.max_sessions, DEFAULTS.scrapling.max_sessions)} max · guidance ${booleanValue(scrapling.inject_guidance, DEFAULTS.scrapling.inject_guidance) ? 'on' : 'off'}`,
    },
  ]

  return (
    <nav className="br-cfg-nav" aria-label="Browser configuration sections">
      <div className="br-cfg-nav-head">
        <p className="br-cfg-nav-label">Browser settings</p>
        <p>Settings are grouped by when and where they apply.</p>
      </div>
      <ul className="br-cfg-nav-list">
        {sections.map((section) => {
          const active = section.id === selection
          return (
            <li key={section.id}>
              <button
                type="button"
                className={`br-cfg-nav-row${active ? ' active' : ''}`}
                data-config-section={section.id}
                aria-current={active ? 'page' : undefined}
                onClick={() => onSelect(section.id)}
              >
                <span className="br-cfg-nav-copy">
                  <span className="br-cfg-nav-name">{section.label}</span>
                  <span className="br-cfg-nav-description">{section.description}</span>
                  <span className="br-cfg-nav-meta">{section.summary}</span>
                </span>
                <ChevronLeftIcon className="br-cfg-nav-chevron" />
              </button>
            </li>
          )
        })}
      </ul>
      <div className="br-cfg-nav-foot">
        <span className="br-cfg-nav-foot-dot" aria-hidden />
        Changes to limits and timeouts hot-apply after saving.
      </div>
    </nav>
  )
}

function EditorHeader({
  title,
  titleId,
  description,
  narrow,
  onBack,
}: {
  title: string
  titleId: string
  description: string
  narrow: boolean
  onBack: () => void
}) {
  return (
    <header className="br-cfg-editor-head">
      {narrow ? (
        <button type="button" className="br-cfg-back" onClick={onBack} aria-label="Back to configuration sections">
          <ChevronLeftIcon />
        </button>
      ) : null}
      <GlobeIcon className="br-cfg-editor-icon" />
      <div className="br-cfg-editor-title">
        <h3 id={titleId}>{title}</h3>
        <p>{description}</p>
      </div>
    </header>
  )
}

export function BrowserConfigEditor({
  selection,
  value,
  errors,
  narrow,
  onBack,
  onChange,
}: {
  selection: SectionId
  value: JsonObject
  errors: ConfigFormProps['errors']
  narrow: boolean
  onBack: () => void
  onChange: (value: JsonObject) => void
}) {
  const setString = (field: string, raw: string) => {
    const next = { ...value }
    if (raw === '') delete next[field]
    else next[field] = raw
    onChange(next)
  }

  const setNumber = (field: string, raw: string) => {
    const next = { ...value }
    if (raw.trim() === '') delete next[field]
    else {
      const parsed = Number(raw)
      next[field] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
    }
    onChange(next)
  }

  const setBoolean = (field: string, nextValue: boolean | string) => {
    onChange({ ...value, [field]: nextValue })
  }

  const setNestedString = (parent: string, field: string, raw: string) => {
    const block = asObject(value[parent])
    if (raw === '') delete block[field]
    else block[field] = raw
    onChange({ ...value, [parent]: block })
  }

  const setNestedNumber = (parent: string, field: string, raw: string) => {
    const block = asObject(value[parent])
    if (raw.trim() === '') delete block[field]
    else {
      const parsed = Number(raw)
      block[field] = Number.isInteger(parsed) && parsed >= 0 ? parsed : raw
    }
    onChange({ ...value, [parent]: block })
  }

  const setNestedBoolean = (parent: string, field: string, nextValue: boolean | string) => {
    onChange({
      ...value,
      [parent]: { ...asObject(value[parent]), [field]: nextValue },
    })
  }

  const setDeepString = (parent: string, child: string, field: string, raw: string) => {
    const block = asObject(value[parent])
    const nested = asObject(block[child])
    if (raw === '') delete nested[field]
    else nested[field] = raw
    block[child] = nested
    onChange({ ...value, [parent]: block })
  }

  const setDeepBoolean = (parent: string, child: string, field: string, nextValue: boolean | string) => {
    const block = asObject(value[parent])
    block[child] = { ...asObject(block[child]), [field]: nextValue }
    onChange({ ...value, [parent]: block })
  }

  const titles: Record<SectionId, { title: string; description: string }> = {
    launch: {
      title: 'Launch and sessions',
      description: 'Choose how Chromium starts and how many sessions can run.',
    },
    viewport: {
      title: 'Viewport and screenshots',
      description: 'Set the canvas used by new sessions and image capture.',
    },
    limits: {
      title: 'Capture limits',
      description: 'Bound live history and serialized page snapshots.',
    },
    behavior: {
      title: 'Runtime behavior',
      description: 'Control timeouts, idle cleanup, and allowed destinations.',
    },
    access: {
      title: 'Access policy',
      description: 'Control sensitive browser data and permissions by origin.',
    },
    scraping: {
      title: 'Scraping',
      description: 'Configure the isolated Scrapling-compatible runtime.',
    },
  }
  const titleId = `br-cfg-editor-title-${selection}`

  return (
    <section className="br-cfg-editor" data-section={selection} tabIndex={-1} aria-labelledby={titleId}>
      <EditorHeader {...titles[selection]} titleId={titleId} narrow={narrow} onBack={onBack} />
      <div className="br-cfg-editor-scroll">
        {selection === 'launch' ? (
          <>
            <ConfigSection
              title="Chromium process"
              description="Leave paths empty to use an auto-detected browser and an ephemeral profile."
            >
              <SettingsList className="br-cfg-section-list">
                <TextField
                  field="executable"
                  label="Browser executable"
                  value={stringValue(value.executable)}
                  placeholder="Auto-detect Chrome, Chromium, or Edge"
                  hint="Use an absolute path only when auto-detection cannot find the intended browser."
                  error={fieldError(errors, 'executable')}
                  onChange={(next) => setString('executable', next)}
                />
                <TextField
                  field="user_data_dir"
                  label="Profile directory"
                  value={stringValue(value.user_data_dir)}
                  placeholder="Ephemeral profile per session"
                  hint="A persistent directory keeps cookies and logins. All sessions share the same profile."
                  error={fieldError(errors, 'user_data_dir')}
                  onChange={(next) => setString('user_data_dir', next)}
                />
              </SettingsList>
            </ConfigSection>
            <ConfigSection
              title="Session policy"
              description="Launch settings apply to sessions started after the configuration is saved."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="max_sessions"
                  label="Maximum concurrent sessions"
                  value={value.max_sessions}
                  placeholder={DEFAULTS.max_sessions}
                  min={1}
                  error={fieldError(errors, 'max_sessions')}
                  onChange={(next) => setNumber('max_sessions', next)}
                />
                <CheckField
                  field="headless"
                  label="Launch sessions headless"
                  hint="Turn this off to open visible browser windows on the worker host."
                  value={value.headless}
                  fallback={DEFAULTS.headless}
                  error={fieldError(errors, 'headless')}
                  onChange={(next) => setBoolean('headless', next)}
                />
                <CheckField
                  field="allow_attach"
                  label="Allow attaching to existing browsers"
                  hint="Attached tabs can access the real browser profile and its signed-in sessions."
                  value={value.allow_attach}
                  fallback={DEFAULTS.allow_attach}
                  error={fieldError(errors, 'allow_attach')}
                  onChange={(next) => setBoolean('allow_attach', next)}
                />
              </SettingsList>
              {booleanValue(value.allow_attach, DEFAULTS.allow_attach) ? (
                <div className="br-cfg-warning" role="note">
                  Attach mode is enabled. Only connect to browser instances you trust.
                </div>
              ) : null}
            </ConfigSection>
          </>
        ) : null}

        {selection === 'viewport' ? (
          <>
            <ConfigSection
              title="New session viewport"
              description="These dimensions define the page coordinate space for screenshots, clicks, and scrolls."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="viewport_width"
                  label="Width (pixels)"
                  value={value.viewport_width}
                  placeholder={DEFAULTS.viewport_width}
                  min={320}
                  error={fieldError(errors, 'viewport_width')}
                  onChange={(next) => setNumber('viewport_width', next)}
                />
                <NumberField
                  field="viewport_height"
                  label="Height (pixels)"
                  value={value.viewport_height}
                  placeholder={DEFAULTS.viewport_height}
                  min={240}
                  error={fieldError(errors, 'viewport_height')}
                  onChange={(next) => setNumber('viewport_height', next)}
                />
              </SettingsList>
              <div className="br-cfg-preview">
                <div
                  className="br-cfg-preview-frame"
                  style={{
                    aspectRatio: `${numberValue(value.viewport_width, DEFAULTS.viewport_width)} / ${numberValue(value.viewport_height, DEFAULTS.viewport_height)}`,
                  }}
                >
                  <span>
                    {numberValue(value.viewport_width, DEFAULTS.viewport_width)} ×{' '}
                    {numberValue(value.viewport_height, DEFAULTS.viewport_height)}
                  </span>
                </div>
                <p>Aspect-ratio preview for newly launched sessions.</p>
              </div>
            </ConfigSection>
            <ConfigSection
              title="Screenshot capture"
              description="Balance image detail against artifact size and transfer time."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="screenshot_quality"
                  label="JPEG quality"
                  value={value.screenshot_quality}
                  placeholder={DEFAULTS.screenshot_quality}
                  min={1}
                  max={100}
                  hint="Use a value from 1 to 100. The default is tuned for compact diagnostic screenshots."
                  error={fieldError(errors, 'screenshot_quality')}
                  onChange={(next) => setNumber('screenshot_quality', next)}
                />
              </SettingsList>
            </ConfigSection>
          </>
        ) : null}

        {selection === 'limits' ? (
          <>
            <ConfigSection
              title="Live history buffers"
              description="Each session keeps the most recent console and network events in memory."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="console_buffer"
                  label="Console entries per session"
                  value={value.console_buffer}
                  placeholder={DEFAULTS.console_buffer}
                  hint={`${formatCount(numberValue(value.console_buffer, DEFAULTS.console_buffer))} entries retained.`}
                  error={fieldError(errors, 'console_buffer')}
                  onChange={(next) => setNumber('console_buffer', next)}
                />
                <NumberField
                  field="network_buffer"
                  label="Network entries per session"
                  value={value.network_buffer}
                  placeholder={DEFAULTS.network_buffer}
                  hint={`${formatCount(numberValue(value.network_buffer, DEFAULTS.network_buffer))} requests retained.`}
                  error={fieldError(errors, 'network_buffer')}
                  onChange={(next) => setNumber('network_buffer', next)}
                />
              </SettingsList>
            </ConfigSection>
            <ConfigSection
              title="Snapshot budget"
              description="Large pages are truncated after this many serialized accessibility nodes."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="max_snapshot_nodes"
                  label="Maximum snapshot nodes"
                  value={value.max_snapshot_nodes}
                  placeholder={DEFAULTS.max_snapshot_nodes}
                  hint={`${formatCount(numberValue(value.max_snapshot_nodes, DEFAULTS.max_snapshot_nodes))} nodes maximum per snapshot.`}
                  error={fieldError(errors, 'max_snapshot_nodes')}
                  onChange={(next) => setNumber('max_snapshot_nodes', next)}
                />
              </SettingsList>
            </ConfigSection>
          </>
        ) : null}

        {selection === 'behavior' ? (
          <>
            <ConfigSection
              title="Operation timeouts"
              description="Callers may request a shorter timeout, but never one above the configured ceiling."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field="default_timeout_ms"
                  label="Default timeout (milliseconds)"
                  value={value.default_timeout_ms}
                  placeholder={DEFAULTS.default_timeout_ms}
                  hint={`Currently ${formatDuration(numberValue(value.default_timeout_ms, DEFAULTS.default_timeout_ms))}.`}
                  error={fieldError(errors, 'default_timeout_ms')}
                  onChange={(next) => setNumber('default_timeout_ms', next)}
                />
                <NumberField
                  field="max_timeout_ms"
                  label="Maximum timeout (milliseconds)"
                  value={value.max_timeout_ms}
                  placeholder={DEFAULTS.max_timeout_ms}
                  hint={`Currently ${formatDuration(numberValue(value.max_timeout_ms, DEFAULTS.max_timeout_ms))}.`}
                  error={fieldError(errors, 'max_timeout_ms')}
                  onChange={(next) => setNumber('max_timeout_ms', next)}
                />
                <NumberField
                  field="idle_stop_ms"
                  label="Idle session cleanup (milliseconds)"
                  value={value.idle_stop_ms}
                  placeholder={DEFAULTS.idle_stop_ms}
                  hint={`Currently ${formatDuration(numberValue(value.idle_stop_ms, DEFAULTS.idle_stop_ms))}. Set 0 to disable automatic cleanup.`}
                  error={fieldError(errors, 'idle_stop_ms')}
                  onChange={(next) => setNumber('idle_stop_ms', next)}
                />
              </SettingsList>
            </ConfigSection>
            <ConfigSection
              title="Navigation policy"
              description="Only URLs using these schemes can be opened by browser::navigate."
            >
              <SettingsList className="br-cfg-section-list">
                <SchemesField
                  value={
                    Array.isArray(value.allowed_schemes)
                      ? value.allowed_schemes.filter((scheme): scheme is string => typeof scheme === 'string')
                      : [...DEFAULTS.allowed_schemes]
                  }
                  error={fieldError(errors, 'allowed_schemes')}
                  onChange={(schemes) => onChange({ ...value, allowed_schemes: schemes })}
                />
              </SettingsList>
            </ConfigSection>
          </>
        ) : null}

        {selection === 'access' ? (
          <>
            <ConfigSection
              title="Sensitive browser data"
              description="These switches apply to every session and caller unless the capability is disabled here."
            >
              <SettingsList className="br-cfg-section-list">
                <CheckField
                  field="allow_history_access"
                  label="Expose visited-page history"
                  hint="Allows browser::history::list to return the session's visited pages."
                  value={value.allow_history_access}
                  fallback={DEFAULTS.allow_history_access}
                  error={fieldError(errors, 'allow_history_access')}
                  onChange={(next) => setBoolean('allow_history_access', next)}
                />
                <CheckField
                  field="allow_cookie_import"
                  label="Allow cookie imports"
                  hint="Allows browser::cookies::set to import cookies into a session."
                  value={value.allow_cookie_import}
                  fallback={DEFAULTS.allow_cookie_import}
                  error={fieldError(errors, 'allow_cookie_import')}
                  onChange={(next) => setBoolean('allow_cookie_import', next)}
                />
              </SettingsList>
            </ConfigSection>
            <ConfigSection
              title="Origin permissions"
              description="Exact origins take precedence over bare hostnames, followed by the fallback policy. Missing decisions allow the capability."
            >
              <DefaultOriginPolicy value={value} errors={errors} onChange={onChange} />
              <OriginPolicies value={value} errors={errors} onChange={onChange} />
            </ConfigSection>
          </>
        ) : null}

        {selection === 'scraping' ? (
          <div data-field="scrapling" tabIndex={-1}>
            <ConfigSection
              title="Native runtime"
              description="The scraping runtime is isolated from interactive browser sessions and has its own Chromium and network policy."
            >
              <SettingsList className="br-cfg-section-list">
                <SelectField
                  field={['scrapling', 'security_mode']}
                  label="Security mode"
                  value={stringValue(asObject(value.scrapling).security_mode, DEFAULTS.scrapling.security_mode)}
                  options={[
                    {
                      value: 'safe',
                      label: 'Safe — bounded adaptive storage',
                    },
                    {
                      value: 'compat',
                      label: 'Compat — unbounded adaptive storage',
                    },
                  ]}
                  hint="Compat is only available on supported Linux x86_64 or arm64 builds."
                  error={fieldError(errors, ['scrapling', 'security_mode'])}
                  onChange={(next) => setNestedString('scrapling', 'security_mode', next)}
                />
                <TextField
                  field={['scrapling', 'chromium_executable']}
                  label="Scraping Chromium executable"
                  value={stringValue(asObject(value.scrapling).chromium_executable)}
                  placeholder="Auto-detect Chromium"
                  hint="Independent from the executable used by interactive sessions."
                  error={fieldError(errors, ['scrapling', 'chromium_executable'])}
                  onChange={(next) => setNestedString('scrapling', 'chromium_executable', next)}
                />
                <CheckField
                  field={['scrapling', 'allow_loopback']}
                  label="Allow loopback targets"
                  hint="Permit scraping requests to localhost and other loopback addresses. Keep this off for shared workers."
                  value={asObject(value.scrapling).allow_loopback}
                  fallback={DEFAULTS.scrapling.allow_loopback}
                  error={fieldError(errors, ['scrapling', 'allow_loopback'])}
                  onChange={(next) => setNestedBoolean('scrapling', 'allow_loopback', next)}
                />
              </SettingsList>
              {stringValue(asObject(value.scrapling).security_mode, DEFAULTS.scrapling.security_mode) === 'compat' ? (
                <div className="br-cfg-warning" role="note">
                  Compat removes the adaptive-storage quota and is rejected on unsupported targets.
                </div>
              ) : null}
            </ConfigSection>

            <ConfigSection
              title="Fetch defaults"
              description="Defaults used when a browser::* scraping call does not supply its own value."
            >
              <SettingsList className="br-cfg-section-list">
                <TextField
                  field={['scrapling', 'defaults', 'impersonate']}
                  label="Browser fingerprint"
                  value={stringValue(
                    asObject(asObject(value.scrapling).defaults).impersonate,
                    DEFAULTS.scrapling.defaults.impersonate,
                  )}
                  placeholder={DEFAULTS.scrapling.defaults.impersonate}
                  hint="For example chrome or firefox."
                  error={fieldError(errors, ['scrapling', 'defaults', 'impersonate'])}
                  onChange={(next) => setDeepString('scrapling', 'defaults', 'impersonate', next)}
                />
                <TextField
                  field={['scrapling', 'defaults', 'proxy']}
                  label="Proxy URL"
                  value={stringValue(asObject(asObject(value.scrapling).defaults).proxy)}
                  placeholder="Direct connection"
                  hint="Leave empty to connect without a proxy."
                  error={fieldError(errors, ['scrapling', 'defaults', 'proxy'])}
                  onChange={(next) => setDeepString('scrapling', 'defaults', 'proxy', next)}
                />
                <CheckField
                  field={['scrapling', 'defaults', 'headless']}
                  label="Run fetches headless"
                  hint="Launch native browser-backed fetches without a visible window."
                  value={asObject(asObject(value.scrapling).defaults).headless}
                  fallback={DEFAULTS.scrapling.defaults.headless}
                  error={fieldError(errors, ['scrapling', 'defaults', 'headless'])}
                  onChange={(next) => setDeepBoolean('scrapling', 'defaults', 'headless', next)}
                />
                <CheckField
                  field={['scrapling', 'defaults', 'network_idle']}
                  label="Wait for network idle"
                  hint="Wait for late network activity before returning browser-backed fetches."
                  value={asObject(asObject(value.scrapling).defaults).network_idle}
                  fallback={DEFAULTS.scrapling.defaults.network_idle}
                  error={fieldError(errors, ['scrapling', 'defaults', 'network_idle'])}
                  onChange={(next) => setDeepBoolean('scrapling', 'defaults', 'network_idle', next)}
                />
                <CheckField
                  field={['scrapling', 'defaults', 'include_html']}
                  label="Include source HTML"
                  hint="Return raw HTML alongside extracted content by default."
                  value={asObject(asObject(value.scrapling).defaults).include_html}
                  fallback={DEFAULTS.scrapling.defaults.include_html}
                  error={fieldError(errors, ['scrapling', 'defaults', 'include_html'])}
                  onChange={(next) => setDeepBoolean('scrapling', 'defaults', 'include_html', next)}
                />
              </SettingsList>
            </ConfigSection>

            <ConfigSection
              title="Capacity and adaptive storage"
              description="Bound concurrent work, retained sessions, and the adaptive selector database."
            >
              <SettingsList className="br-cfg-section-list">
                <NumberField
                  field={['scrapling', 'max_bulk_concurrency']}
                  label="Maximum bulk concurrency"
                  value={asObject(value.scrapling).max_bulk_concurrency}
                  placeholder={DEFAULTS.scrapling.max_bulk_concurrency}
                  hint="Maximum concurrent requests inside one bulk operation."
                  error={fieldError(errors, ['scrapling', 'max_bulk_concurrency'])}
                  onChange={(next) => setNestedNumber('scrapling', 'max_bulk_concurrency', next)}
                />
                <NumberField
                  field={['scrapling', 'max_sessions']}
                  label="Maximum scraping sessions"
                  value={asObject(value.scrapling).max_sessions}
                  placeholder={DEFAULTS.scrapling.max_sessions}
                  hint="Independent from interactive browser sessions."
                  error={fieldError(errors, ['scrapling', 'max_sessions'])}
                  onChange={(next) => setNestedNumber('scrapling', 'max_sessions', next)}
                />
                <NumberField
                  field={['scrapling', 'session_idle_timeout_s']}
                  label="Session idle timeout (seconds)"
                  value={asObject(value.scrapling).session_idle_timeout_s}
                  placeholder={DEFAULTS.scrapling.session_idle_timeout_s}
                  hint="Startup setting for reclaiming inactive scraping sessions."
                  error={fieldError(errors, ['scrapling', 'session_idle_timeout_s'])}
                  onChange={(next) => setNestedNumber('scrapling', 'session_idle_timeout_s', next)}
                />
                <NumberField
                  field={['scrapling', 'adaptive_max_bytes']}
                  label="Adaptive storage limit (bytes)"
                  value={asObject(value.scrapling).adaptive_max_bytes}
                  placeholder={DEFAULTS.scrapling.adaptive_max_bytes}
                  hint={`Currently ${formatBytes(numberValue(asObject(value.scrapling).adaptive_max_bytes, DEFAULTS.scrapling.adaptive_max_bytes))}. Ignored in compat mode.`}
                  error={fieldError(errors, ['scrapling', 'adaptive_max_bytes'])}
                  onChange={(next) => setNestedNumber('scrapling', 'adaptive_max_bytes', next)}
                />
                <TextField
                  field={['scrapling', 'adaptive_storage_path']}
                  label="Adaptive storage path"
                  value={stringValue(
                    asObject(value.scrapling).adaptive_storage_path,
                    DEFAULTS.scrapling.adaptive_storage_path,
                  )}
                  placeholder={DEFAULTS.scrapling.adaptive_storage_path}
                  hint="Startup setting. Relative paths resolve from the project root."
                  error={fieldError(errors, ['scrapling', 'adaptive_storage_path'])}
                  onChange={(next) => setNestedString('scrapling', 'adaptive_storage_path', next)}
                />
              </SettingsList>
            </ConfigSection>

            <ConfigSection
              title="Agent guidance"
              description="Whether the worker teaches agents its scraping surface via the system prompt."
            >
              <SettingsList className="br-cfg-section-list">
                <CheckField
                  field={['scrapling', 'inject_guidance']}
                  label="Inject scraping guidance into agent system prompts"
                  hint="Hot-applies on save: turning this off unbinds the pre-generate hook immediately, without a worker restart."
                  value={asObject(value.scrapling).inject_guidance}
                  fallback={DEFAULTS.scrapling.inject_guidance}
                  error={fieldError(errors, ['scrapling', 'inject_guidance'])}
                  onChange={(next) => setNestedBoolean('scrapling', 'inject_guidance', next)}
                />
              </SettingsList>
            </ConfigSection>
          </div>
        ) : null}
      </div>
    </section>
  )
}

export function BrowserConfigForm(props: ConfigFormProps) {
  const value = browserConfigurationValue(props.value)
  const [rootRef, narrow] = useContainerNarrow(CONFIG_NARROW_BELOW)
  const [selection, setSelection] = useState<SectionId>('launch')
  const [narrowPane, setNarrowPane] = useState<'nav' | 'editor'>('nav')
  const domRef = useRef<HTMLDivElement | null>(null)
  const pendingNarrowFocusRef = useRef<'nav' | 'editor' | null>(null)
  const focusFieldName = props.focusField?.join('.') ?? ''
  const focusRoot = props.focusField?.[0] ?? ''

  const setRoot = useCallback(
    (node: HTMLDivElement | null) => {
      rootRef(node)
      domRef.current = node
    },
    [rootRef],
  )

  const choose = (section: SectionId) => {
    if (narrow) pendingNarrowFocusRef.current = 'editor'
    setSelection(section)
    setNarrowPane('editor')
  }

  const showNavigation = () => {
    pendingNarrowFocusRef.current = 'nav'
    setNarrowPane('nav')
  }

  useEffect(() => {
    if (!focusRoot) return
    setSelection(FIELD_SECTION[focusRoot] ?? 'launch')
    setNarrowPane('editor')
  }, [focusRoot])

  useEffect(() => {
    if (!focusFieldName || !focusRoot || !domRef.current) return
    if (selection !== (FIELD_SECTION[focusRoot] ?? 'launch')) return
    const target =
      domRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusFieldName)}"]`) ??
      domRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(focusRoot)}"]`)
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [focusFieldName, focusRoot, selection])

  useEffect(() => {
    if (!narrow || pendingNarrowFocusRef.current !== narrowPane || !domRef.current) return
    const frame = requestAnimationFrame(() => {
      if (focusBrowserNarrowPane(domRef.current, narrowPane, selection)) {
        pendingNarrowFocusRef.current = null
      }
    })
    return () => cancelAnimationFrame(frame)
  }, [narrow, narrowPane, selection])

  const showNav = !narrow || narrowPane === 'nav'
  const showEditor = !narrow || narrowPane === 'editor'

  return (
    <div className={`br-cfg${narrow ? ' narrow' : ''}`} ref={setRoot}>
      <div className="br-cfg-workbench">
        {showNav ? <ConfigNav value={value} selection={selection} onSelect={choose} /> : null}
        {showEditor ? (
          <BrowserConfigEditor
            selection={selection}
            value={value}
            errors={props.errors}
            narrow={narrow}
            onBack={showNavigation}
            onChange={(nextValue) => props.onChange(migrateBrowserConfiguration(props.value, nextValue))}
          />
        ) : null}
      </div>

      {props.errors && props.errors.size > 0 ? (
        <StatusPanel
          variant="alert"
          headline="Configuration needs attention"
          detail={`${props.errors.size} validation ${props.errors.size === 1 ? 'error is' : 'errors are'} marked in the form.`}
          className="br-cfg-status"
        />
      ) : null}
    </div>
  )
}
