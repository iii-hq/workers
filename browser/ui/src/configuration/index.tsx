/**
 * Purpose-built configuration editor for the browser worker. The Console
 * retains ownership of dirty tracking, validation, save, reset, and the
 * unsaved-change guard; this component only edits the JSON draft.
 */

import {
  type ConfigFormProps,
  Input,
  type JsonValue,
  StatusPanel,
} from '@iii-dev/console-ui'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { ChevronLeftIcon, GlobeIcon, useContainerNarrow } from '../lib/widgets'

type JsonObject = { [key: string]: JsonValue }
type SectionId = 'launch' | 'viewport' | 'limits' | 'behavior'

const CONFIG_NARROW_BELOW = 660
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
  allowed_schemes: ['http', 'https'],
  max_snapshot_nodes: 2_000,
  allow_attach: false,
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
}

function asObject(value: JsonValue | undefined): JsonObject {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? { ...value }
    : {}
}

function stringValue(value: JsonValue | undefined, fallback = ''): string {
  return typeof value === 'string' ? value : fallback
}

function numberValue(value: JsonValue | undefined, fallback: number): number {
  return typeof value === 'number' ? value : fallback
}

function booleanValue(
  value: JsonValue | undefined,
  fallback: boolean,
): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function pointer(field: string) {
  return `/${field.replaceAll('~', '~0').replaceAll('/', '~1')}`
}

function fieldError(errors: ConfigFormProps['errors'], field: string) {
  const base = pointer(field)
  return (
    errors?.get(base) ??
    [...(errors?.entries() ?? [])].find(([path]) =>
      path.startsWith(`${base}/`),
    )?.[1]
  )
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

function Field({
  label,
  hint,
  error,
  children,
}: {
  label: ReactNode
  hint?: ReactNode
  error?: string
  children: ReactNode
}) {
  return (
    <div className="br-cfg-field">
      <div className="br-cfg-field-label">{label}</div>
      {children}
      {error ? (
        <p className="br-cfg-error" role="alert">
          {error}
        </p>
      ) : null}
      {hint ? <p className="br-cfg-hint">{hint}</p> : null}
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
  field: string
  label: string
  value: string
  placeholder?: string
  hint?: ReactNode
  error?: string
  onChange: (value: string) => void
}) {
  const id = `br-cfg-${field}`
  return (
    <Field
      label={<label htmlFor={id}>{label}</label>}
      hint={hint}
      error={error}
    >
      <Input
        id={id}
        name={field}
        data-field={field}
        className="br-cfg-input"
        value={value}
        preserveCase
        spellCheck={false}
        autoComplete="off"
        placeholder={placeholder}
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
  field: string
  label: string
  value: JsonValue | undefined
  placeholder: number
  min?: number
  max?: number
  hint?: ReactNode
  error?: string
  onChange: (raw: string) => void
}) {
  const id = `br-cfg-${field}`
  return (
    <Field
      label={<label htmlFor={id}>{label}</label>}
      hint={hint}
      error={error}
    >
      <Input
        id={id}
        name={field}
        data-field={field}
        className="br-cfg-input"
        type="number"
        min={min}
        max={max}
        inputMode="numeric"
        value={typeof value === 'number' ? String(value) : ''}
        preserveCase
        placeholder={String(placeholder)}
        onChange={onChange}
      />
    </Field>
  )
}

function CheckField({
  field,
  label,
  hint,
  checked,
  onChange,
}: {
  field: string
  label: string
  hint: ReactNode
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  const id = `br-cfg-${field}`
  return (
    <div className="br-cfg-check-field">
      <label className="br-cfg-check-row" htmlFor={id}>
        <input
          id={id}
          name={field}
          data-field={field}
          type="checkbox"
          checked={checked}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span>{label}</span>
      </label>
      <p className="br-cfg-hint">{hint}</p>
    </div>
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
  const [draft, setDraft] = useState(canonical)

  useEffect(() => setDraft(canonical), [canonical])

  const commit = () => {
    onChange(
      draft
        .split(',')
        .map((scheme) => scheme.trim())
        .filter(Boolean),
    )
  }

  return (
    <Field
      label={
        <label htmlFor="br-cfg-allowed_schemes">Allowed URL schemes</label>
      }
      hint="Enter a comma-separated list without ://. Keep this list as narrow as your workflows allow."
      error={error}
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
        onChange={setDraft}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key !== 'Enter') return
          event.preventDefault()
          commit()
          event.currentTarget.blur()
        }}
      />
    </Field>
  )
}

function SectionHeader({
  title,
  description,
}: {
  title: string
  description: string
}) {
  return (
    <div className="br-cfg-section-head">
      <div>
        <h4>{title}</h4>
        <p>{description}</p>
      </div>
    </div>
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
  const consoleBuffer = numberValue(
    value.console_buffer,
    DEFAULTS.console_buffer,
  )
  const networkBuffer = numberValue(
    value.network_buffer,
    DEFAULTS.network_buffer,
  )
  const timeout = numberValue(
    value.default_timeout_ms,
    DEFAULTS.default_timeout_ms,
  )
  const idle = numberValue(value.idle_stop_ms, DEFAULTS.idle_stop_ms)

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
                aria-current={active ? 'page' : undefined}
                onClick={() => onSelect(section.id)}
              >
                <span className="br-cfg-nav-copy">
                  <span className="br-cfg-nav-name">{section.label}</span>
                  <span className="br-cfg-nav-description">
                    {section.description}
                  </span>
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
  description,
  narrow,
  onBack,
}: {
  title: string
  description: string
  narrow: boolean
  onBack: () => void
}) {
  return (
    <header className="br-cfg-editor-head">
      {narrow ? (
        <button
          type="button"
          className="br-cfg-back"
          onClick={onBack}
          aria-label="Back to configuration sections"
        >
          <ChevronLeftIcon />
        </button>
      ) : null}
      <GlobeIcon className="br-cfg-editor-icon" />
      <div className="br-cfg-editor-title">
        <h3>{title}</h3>
        <p>{description}</p>
      </div>
    </header>
  )
}

function ConfigEditor({
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
      if (!Number.isInteger(parsed) || parsed < 0) return
      next[field] = parsed
    }
    onChange(next)
  }

  const setBoolean = (field: string, checked: boolean) => {
    onChange({ ...value, [field]: checked })
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
  }

  return (
    <section className="br-cfg-editor" data-section={selection} tabIndex={-1}>
      <EditorHeader {...titles[selection]} narrow={narrow} onBack={onBack} />
      <div className="br-cfg-editor-scroll">
        {selection === 'launch' ? (
          <>
            <section className="br-cfg-section">
              <SectionHeader
                title="Chromium process"
                description="Leave paths empty to use an auto-detected browser and an ephemeral profile."
              />
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
            </section>
            <section className="br-cfg-section">
              <SectionHeader
                title="Session policy"
                description="Launch settings apply to sessions started after the configuration is saved."
              />
              <NumberField
                field="max_sessions"
                label="Maximum concurrent sessions"
                value={value.max_sessions}
                placeholder={DEFAULTS.max_sessions}
                min={1}
                error={fieldError(errors, 'max_sessions')}
                onChange={(next) => setNumber('max_sessions', next)}
              />
              <div className="br-cfg-check-grid">
                <CheckField
                  field="headless"
                  label="Launch sessions headless"
                  hint="Turn this off to open visible browser windows on the worker host."
                  checked={booleanValue(value.headless, DEFAULTS.headless)}
                  onChange={(next) => setBoolean('headless', next)}
                />
                <CheckField
                  field="allow_attach"
                  label="Allow attaching to existing browsers"
                  hint="Attached tabs can access the real browser profile and its signed-in sessions."
                  checked={booleanValue(
                    value.allow_attach,
                    DEFAULTS.allow_attach,
                  )}
                  onChange={(next) => setBoolean('allow_attach', next)}
                />
              </div>
              {booleanValue(value.allow_attach, DEFAULTS.allow_attach) ? (
                <div className="br-cfg-warning" role="note">
                  Attach mode is enabled. Only connect to browser instances you
                  trust.
                </div>
              ) : null}
            </section>
          </>
        ) : null}

        {selection === 'viewport' ? (
          <>
            <section className="br-cfg-section">
              <SectionHeader
                title="New session viewport"
                description="These dimensions define the page coordinate space for screenshots, clicks, and scrolls."
              />
              <div className="br-cfg-field-grid">
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
              </div>
              <div className="br-cfg-preview">
                <div
                  className="br-cfg-preview-frame"
                  style={{
                    aspectRatio: `${numberValue(value.viewport_width, DEFAULTS.viewport_width)} / ${numberValue(value.viewport_height, DEFAULTS.viewport_height)}`,
                  }}
                >
                  <span>
                    {numberValue(value.viewport_width, DEFAULTS.viewport_width)}{' '}
                    ×{' '}
                    {numberValue(
                      value.viewport_height,
                      DEFAULTS.viewport_height,
                    )}
                  </span>
                </div>
                <p>Aspect-ratio preview for newly launched sessions.</p>
              </div>
            </section>
            <section className="br-cfg-section">
              <SectionHeader
                title="Screenshot capture"
                description="Balance image detail against artifact size and transfer time."
              />
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
            </section>
          </>
        ) : null}

        {selection === 'limits' ? (
          <>
            <section className="br-cfg-section">
              <SectionHeader
                title="Live history buffers"
                description="Each session keeps the most recent console and network events in memory."
              />
              <div className="br-cfg-field-grid">
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
              </div>
            </section>
            <section className="br-cfg-section">
              <SectionHeader
                title="Snapshot budget"
                description="Large pages are truncated after this many serialized accessibility nodes."
              />
              <NumberField
                field="max_snapshot_nodes"
                label="Maximum snapshot nodes"
                value={value.max_snapshot_nodes}
                placeholder={DEFAULTS.max_snapshot_nodes}
                hint={`${formatCount(numberValue(value.max_snapshot_nodes, DEFAULTS.max_snapshot_nodes))} nodes maximum per snapshot.`}
                error={fieldError(errors, 'max_snapshot_nodes')}
                onChange={(next) => setNumber('max_snapshot_nodes', next)}
              />
            </section>
          </>
        ) : null}

        {selection === 'behavior' ? (
          <>
            <section className="br-cfg-section">
              <SectionHeader
                title="Operation timeouts"
                description="Callers may request a shorter timeout, but never one above the configured ceiling."
              />
              <div className="br-cfg-field-grid">
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
              </div>
              <NumberField
                field="idle_stop_ms"
                label="Idle session cleanup (milliseconds)"
                value={value.idle_stop_ms}
                placeholder={DEFAULTS.idle_stop_ms}
                hint={`Currently ${formatDuration(numberValue(value.idle_stop_ms, DEFAULTS.idle_stop_ms))}. Set 0 to disable automatic cleanup.`}
                error={fieldError(errors, 'idle_stop_ms')}
                onChange={(next) => setNumber('idle_stop_ms', next)}
              />
            </section>
            <section className="br-cfg-section">
              <SectionHeader
                title="Navigation policy"
                description="Only URLs using these schemes can be opened by browser::navigate."
              />
              <SchemesField
                value={
                  Array.isArray(value.allowed_schemes)
                    ? value.allowed_schemes.filter(
                        (scheme): scheme is string =>
                          typeof scheme === 'string',
                      )
                    : [...DEFAULTS.allowed_schemes]
                }
                error={fieldError(errors, 'allowed_schemes')}
                onChange={(schemes) =>
                  onChange({ ...value, allowed_schemes: schemes })
                }
              />
            </section>
          </>
        ) : null}
      </div>
    </section>
  )
}

export function BrowserConfigForm(props: ConfigFormProps) {
  const value = asObject(props.value)
  const [rootRef, narrow] = useContainerNarrow(CONFIG_NARROW_BELOW)
  const [selection, setSelection] = useState<SectionId>('launch')
  const [narrowPane, setNarrowPane] = useState<'nav' | 'editor'>('nav')
  const domRef = useRef<HTMLDivElement | null>(null)
  const focusKey = props.focusField?.join('/') ?? ''

  const setRoot = (node: HTMLDivElement | null) => {
    rootRef(node)
    domRef.current = node
  }

  const choose = (section: SectionId) => {
    setSelection(section)
    setNarrowPane('editor')
  }

  useEffect(() => {
    const field = props.focusField?.[0]
    if (!field) return
    setSelection(FIELD_SECTION[field] ?? 'launch')
    setNarrowPane('editor')
  }, [focusKey])

  useEffect(() => {
    if (!focusKey || !domRef.current) return
    const field = props.focusField?.[0] ?? focusKey
    const target = domRef.current.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(field)}"]`,
    )
    target?.focus()
    target?.scrollIntoView({ block: 'center' })
  }, [focusKey, selection])

  const showNav = !narrow || narrowPane === 'nav'
  const showEditor = !narrow || narrowPane === 'editor'

  return (
    <div className={`br-cfg${narrow ? ' narrow' : ''}`} ref={setRoot}>
      <div className="br-cfg-workbench">
        {showNav ? (
          <ConfigNav value={value} selection={selection} onSelect={choose} />
        ) : null}
        {showEditor ? (
          <ConfigEditor
            selection={selection}
            value={value}
            errors={props.errors}
            narrow={narrow}
            onBack={() => setNarrowPane('nav')}
            onChange={props.onChange}
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
