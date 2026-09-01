/** Purpose-built configuration for the Console worker. */

import {
  Button,
  type ConfigFormProps,
  type Host,
  Input,
  type JsonValue,
  Select,
  type SelectOption,
  SettingsList,
  SettingsRow,
  SettingsSection,
  Switch,
} from '@iii-dev/console-ui'
import { useEffect, useMemo, useRef, useState } from 'react'
import {
  activeTraceViewId,
  addTraceView,
  asObject,
  newTraceViewId,
  removeTraceView,
  renameTraceView,
  stringList,
  type TraceFilterKey,
  traceFilterList,
  traceViews,
  withActiveTraceView,
  withFollowTurns,
  withTraceFilterList,
} from './preferences'

interface ManifestWorker {
  worker: string
  enabled: boolean
  assets: number
}

interface UiManifest {
  disabled: boolean
  workers?: ManifestWorker[]
}

interface ConfigurationEntry {
  id: string
  name?: string
  description?: string
}

interface ConfigurationList {
  configurations?: ConfigurationEntry[]
}

interface WorkerRow {
  worker: string
  title: string
  description: string
  assets: number
}

const ALL_TRACES = '__all-traces__'

function disabledWorkersOf(value: JsonValue): string[] {
  return stringList(asObject(asObject(value).injectableUi).disabledWorkers)
}

function httpPortOf(value: JsonValue): string {
  const port = asObject(value).http_port
  return typeof port === 'number' || typeof port === 'string' ? String(port) : '3113'
}

function viewOptions(value: JsonValue): SelectOption[] {
  const views = traceViews(value)
  const active = activeTraceViewId(value)
  const options: SelectOption[] = [
    { value: ALL_TRACES, label: 'All traces' },
    ...views.map((view) => ({ value: view.id, label: view.name })),
  ]
  if (active && !views.some((view) => view.id === active)) {
    options.push({ value: active, label: `${active} (missing view)` })
  }
  return options
}

const FILTER_ROWS: ReadonlyArray<{
  key: TraceFilterKey
  label: string
  description: string
  placeholder: string
}> = [
  {
    key: 'hiddenGroups',
    label: 'Hidden span groups',
    description: 'Function groups hidden from trace detail by default.',
    placeholder: 'harness::send',
  },
  {
    key: 'hiddenWorkers',
    label: 'Hidden workers',
    description: 'Workers whose spans stay out of trace detail by default.',
    placeholder: 'context-manager',
  },
  {
    key: 'shownGroups',
    label: 'Shown producer-hidden groups',
    description: 'Groups explicitly shown even when a worker marks them hidden.',
    placeholder: 'worker::function',
  },
  {
    key: 'shownInternal',
    label: 'Shown internal families',
    description: 'Internal span families explicitly revealed in trace detail.',
    placeholder: 'session-events',
  },
]

export function InjectableUiConfigForm(props: ConfigFormProps & { host: Host }) {
  const { host } = props
  const [workers, setWorkers] = useState<WorkerRow[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const value = asObject(props.value)
  const traces = asObject(value.traces)
  const disabled = useMemo(() => disabledWorkersOf(props.value), [props.value])
  const views = traceViews(props.value)
  const active = activeTraceViewId(props.value)
  const selectedView =
    active === undefined
      ? views.some((view) => view.id === 'view-sessions')
        ? 'view-sessions'
        : ALL_TRACES
      : (active ?? ALL_TRACES)

  useEffect(() => {
    let cancelled = false
    Promise.all([
      host.iii.trigger<UiManifest>('console::ui-manifest', {}),
      host.iii
        .trigger<ConfigurationList>('configuration::list', {})
        .catch(() => ({ configurations: [] }) as ConfigurationList),
    ])
      .then(([manifest, configs]) => {
        if (cancelled) return
        const byId = new Map((configs.configurations ?? []).map((configuration) => [configuration.id, configuration]))
        setWorkers(
          (manifest.workers ?? [])
            .filter((worker) => worker.worker !== 'console')
            .map((worker) => {
              const entry = byId.get(worker.worker)
              return {
                worker: worker.worker,
                title: entry?.name || worker.worker,
                description: entry?.description ?? '',
                assets: worker.assets,
              }
            }),
        )
      })
      .catch((error) => {
        if (!cancelled) {
          setLoadError(error instanceof Error ? error.message : String(error))
        }
      })
    return () => {
      cancelled = true
    }
  }, [host])

  const rows = useMemo(() => {
    if (!workers) return null
    const known = new Set(workers.map((worker) => worker.worker))
    const unavailable = disabled
      .filter((worker) => !known.has(worker) && worker !== 'console')
      .map((worker) => ({
        worker,
        title: worker,
        description: 'This worker is not currently registering Console assets.',
        assets: 0,
      }))
    return [...workers, ...unavailable].sort((a, b) => a.title.localeCompare(b.title))
  }, [workers, disabled])

  const toggleWorker = (worker: string, enabled: boolean) => {
    const next = enabled
      ? disabled.filter((candidate) => candidate !== worker)
      : [...disabled.filter((candidate) => candidate !== worker), worker]
    props.onChange({
      ...value,
      injectableUi: {
        ...asObject(value.injectableUi),
        disabledWorkers: next,
      },
    })
  }

  const rootRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (!props.focusField?.length || !rootRef.current) return
    const exact = props.focusField.join('-')
    const target =
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(exact)}"]`) ??
      rootRef.current.querySelector<HTMLElement>(`[data-field="${CSS.escape(props.focusField[0])}"]`)
    target?.scrollIntoView({ block: 'center' })
    const focusable = target?.matches('input, button, [tabindex]')
      ? target
      : target?.querySelector<HTMLElement>('input, button, [tabindex]')
    focusable?.focus()
  }, [props.focusField])

  return (
    <div className="console-ui-form" ref={rootRef}>
      <SettingsSection title="Server" description="Connection settings for this Console instance.">
        <SettingsList>
          <SettingsRow
            data-field="http_port"
            label="HTTP port"
            description="Serves the Console, injected assets, and the WebSocket proxy. Changes apply without restarting the worker."
            meta="Use 0 to let the operating system choose an available port."
            control={
              <Input
                id="console-http-port"
                className="console-ui-port-input"
                type="number"
                inputMode="numeric"
                min={0}
                max={65535}
                step={1}
                value={httpPortOf(props.value)}
                onChange={(next) =>
                  props.onChange({
                    ...value,
                    http_port: /^\d+$/.test(next) ? Number(next) : next,
                  })
                }
                aria-label="HTTP port"
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="traces"
        title="Traces"
        description="Preferences shared by every browser connected to this Console."
      >
        <SettingsList>
          <SettingsRow
            data-field="traces-followTurns"
            label="Follow active turns"
            description="Automatically open the trace for a live turn in the active chat."
            control={
              <Switch
                checked={typeof traces.followTurns === 'boolean' ? traces.followTurns : true}
                onChange={(event) => props.onChange(withFollowTurns(props.value, event.currentTarget.checked))}
                aria-label="Follow active turns"
              />
            }
          />
          <SettingsRow
            data-field="traces-activeViewId"
            label="Active saved view"
            description="Choose the view used when the Traces pane opens."
            control={
              <div className="console-ui-select-control">
                <Select
                  aria-label="Active saved view"
                  value={selectedView}
                  options={viewOptions(props.value)}
                  onChange={(id) => props.onChange(withActiveTraceView(props.value, id === ALL_TRACES ? null : id))}
                />
              </div>
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="traces-views"
        title="Saved trace views"
        description="Rename or remove saved filter snapshots. View details remain unchanged."
        action={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => props.onChange(addTraceView(props.value, newTraceViewId()))}
          >
            Add view
          </Button>
        }
      >
        <SettingsList>
          {views.length === 0 ? (
            <SettingsRow
              label="No saved views"
              description="Add a view here or save the current filters from the Traces pane."
            />
          ) : (
            views.map((view) => (
              <SettingsRow
                key={view.id}
                label={
                  <Input
                    aria-label={`Name for ${view.name}`}
                    value={view.name}
                    onChange={(name) => props.onChange(renameTraceView(props.value, view.id, name))}
                  />
                }
                meta={
                  typeof view.value.groupBy === 'string' && view.value.groupBy !== 'none'
                    ? `Grouped by ${view.value.groupBy}`
                    : 'Ungrouped'
                }
                action={
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => props.onChange(removeTraceView(props.value, view.id))}
                    aria-label={`Remove ${view.name}`}
                  >
                    Remove
                  </Button>
                }
              />
            ))
          )}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="traces-spanFilters"
        title="Span visibility"
        description="Control which spans are hidden or explicitly revealed in trace detail."
      >
        <SettingsList>
          {FILTER_ROWS.map((row) => (
            <SettingsRow
              key={row.key}
              data-field={`traces-spanFilters-${row.key}`}
              layout="stacked"
              label={row.label}
              description={row.description}
              control={
                <StringListEditor
                  label={row.label}
                  placeholder={row.placeholder}
                  values={traceFilterList(props.value, row.key)}
                  onChange={(entries) => props.onChange(withTraceFilterList(props.value, row.key, entries))}
                />
              }
            />
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        data-field="injectableUi"
        title="Worker interface extensions"
        description="Choose which workers can add pages, renderers, and configuration forms to the Console. Changes apply to open tabs after saving."
      >
        {rows === null && loadError === null ? (
          <div className="console-ui-toggle-list" aria-hidden="true">
            <div className="console-ui-toggle-skeleton" />
            <div className="console-ui-toggle-skeleton" />
          </div>
        ) : null}
        {loadError ? (
          <div className="console-ui-form-error" role="alert">
            Could not load the worker list: {loadError}
          </div>
        ) : null}
        {rows?.length === 0 ? (
          <div className="console-ui-form-empty">No worker is currently registering Console assets.</div>
        ) : null}
        {rows && rows.length > 0 ? (
          <SettingsList>
            {rows.map((row) => {
              const enabled = !disabled.includes(row.worker)
              return (
                <SettingsRow
                  key={row.worker}
                  label={row.title}
                  description={row.description || 'Injected Console interface'}
                  meta={
                    row.assets > 0 ? `${row.assets} asset${row.assets === 1 ? '' : 's'}` : 'Not currently registered'
                  }
                  control={
                    <Switch
                      checked={enabled}
                      onChange={(event) => toggleWorker(row.worker, event.currentTarget.checked)}
                      aria-label={`${enabled ? 'Disable' : 'Enable'} ${row.title}`}
                    />
                  }
                />
              )
            })}
          </SettingsList>
        ) : null}
      </SettingsSection>

      <ConfigurationErrors errors={props.errors} />
    </div>
  )
}

function StringListEditor({
  label,
  placeholder,
  values,
  onChange,
}: {
  label: string
  placeholder: string
  values: string[]
  onChange(values: string[]): void
}) {
  const [draft, setDraft] = useState('')
  const addDraft = () => {
    const next = draft.trim()
    if (!next) return
    if (!values.includes(next)) onChange([...values, next])
    setDraft('')
  }

  return (
    <div className="console-ui-string-list">
      {values.length > 0 ? (
        <ul className="console-ui-string-list-values" aria-label={label}>
          {values.map((entry) => (
            <li className="console-ui-string-chip" key={entry}>
              <span>{entry}</span>
              <button
                type="button"
                onClick={() => onChange(values.filter((item) => item !== entry))}
                aria-label={`Remove ${entry} from ${label}`}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      <div className="console-ui-string-list-add">
        <Input
          value={draft}
          onChange={setDraft}
          placeholder={placeholder}
          aria-label={`Add to ${label}`}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              addDraft()
            }
          }}
        />
        <Button type="button" variant="ghost" size="sm" onClick={addDraft}>
          Add
        </Button>
      </div>
    </div>
  )
}

function ConfigurationErrors({ errors }: { errors: ReadonlyMap<string, string> | undefined }) {
  if (!errors || errors.size === 0) return null
  return (
    <div className="console-ui-form-error" role="alert">
      {[...errors.entries()].map(([pointer, message]) => (
        <div key={pointer}>
          {pointer ? `${pointer}: ` : ''}
          {message}
        </div>
      ))}
    </div>
  )
}
