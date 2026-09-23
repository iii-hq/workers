import {
  Button,
  Chip,
  DirectoryPicker,
  Input,
  Selector,
  SettingsDeck,
  SettingsField,
  SettingsList,
  SettingsRow,
  SettingsSection,
  StatusPanel,
  Switch,
} from '@iii-dev/console-ui'
import type { ConfigFormProps, Host } from '@iii-dev/console-ui'
import { Folder, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { client } from '../api'
import { catalogKey, modelGroups, splitKey } from './catalog.js'
import {
  addRepository,
  normalize,
  problems,
  repositoryAt,
  setPath,
  setRepositoryPath,
  setRepositoryWorkers,
} from './form-model.js'
import { WorkerTokens } from './WorkerTokens'
import type { WorkerSuggestion } from './WorkerTokens'
import { useModelCatalog } from './useModelCatalog'

/** Worker names shown on a repository row before the rest fold into a count. */
const SHOWN_WORKERS = 8
const WEEK_MS = 7 * 24 * 3_600_000

/**
 * What the form knows from the worker itself: the workers that failed this
 * week (the names worth suggesting, and the unmapped ones worth pointing
 * out) and which mapped checkouts are actually on disk. Read once per form
 * mount; it is advice, so a failed read shows nothing rather than an error.
 */
function useWorkerFacts(host: Host) {
  const [seen, setSeen] = useState<string[]>([])
  const [onDisk, setOnDisk] = useState<Record<string, boolean>>({})
  useEffect(() => {
    let live = true
    const api = client(host.iii)
    api
      .groups({
        status: ['new', 'investigating', 'diagnosed', 'regressed', 'resolved', 'ignored'],
        since_ms: Date.now() - WEEK_MS,
        limit: 200,
      })
      .then((response) => live && setSeen([...new Set(response.groups.map((group) => group.service_name))].sort()))
      .catch(() => live && setSeen([]))
    api
      .status()
      .then(
        (status) =>
          live &&
          setOnDisk(Object.fromEntries(status.repositories.map((repository) => [repository.id, repository.exists]))),
      )
      .catch(() => live && setOnDisk({}))
    return () => {
      live = false
    }
  }, [host])
  return { seen, onDisk }
}

/**
 * The worker's own settings form.
 *
 * Two things it deliberately does not offer: the ingest budgets — a breaker
 * threshold is not a decision an operator should have to make — and anything
 * about the decision tier, which this version does not have. Keys it does
 * not render are carried through untouched on every save.
 */
export function SentinelConfigForm({
  focusField,
  host,
  value,
  onChange,
}: ConfigFormProps & { host: Host }) {
  const config = normalize(value)
  const { catalog, loading } = useModelCatalog(host)
  const [openRepository, setOpenRepository] = useState<string | null>(null)
  const issues = problems(config)
  const update = (path: string, next: unknown) =>
    onChange(setPath(config, path, next) as ConfigFormProps['value'])

  const investigation = config.investigation as { model?: string; provider?: string }
  const selectedModel = catalogKey(investigation.model, investigation.provider)
  // The two fields move together: a picked row knows its provider, and a
  // cleared field must not leave a provider pointing at nothing.
  const setModel = (model: string, provider: string) =>
    onChange(
      setPath(config, 'investigation', {
        ...investigation,
        model,
        // Absent rather than empty: a blank provider is not a provider, and
        // the worker would carry the empty string into the turn.
        provider: provider || undefined,
      }) as ConfigFormProps['value'],
    )

  const repositories = Array.isArray(config.repositories)
    ? (config.repositories as { id: string; path: string; workers: string[] }[])
    : []
  const ignored = Array.isArray(config.ignore_services) ? (config.ignore_services as string[]) : []
  const joinSeconds =
    ((config.sources as { log?: { join_window_ms?: number } }).log?.join_window_ms ?? 2000) / 1000
  const facts = useWorkerFacts(host)
  const mapped = new Map(repositories.flatMap((repository) => repository.workers.map((worker) => [worker, repository.id])))
  const unmapped = facts.seen.filter((worker) => !mapped.has(worker))
  // What a repository can be given: the workers that failed lately, and the
  // ones another repository holds — picking one of those moves it.
  const suggestionsFor = (id: string): WorkerSuggestion[] => [
    ...unmapped.map((worker) => ({ value: worker, description: 'failed this week, no repository' })),
    ...[...mapped]
      .filter(([, owner]) => owner !== id)
      .map(([worker, owner]) => ({ value: worker, description: `in ${owner}/ — moves here` })),
  ]
  const [folderError, setFolderError] = useState<string | null>(null)
  useEffect(() => setFolderError(null), [openRepository])
  const openAt = (directory: string) => {
    // A folder already mapped opens that repository instead of adding a copy.
    const existing = repositoryAt(config, directory)
    if (existing) {
      setOpenRepository(existing.id)
      return
    }
    const next = addRepository(config, directory)
    onChange(next as ConfigFormProps['value'])
    // Straight into the new repository: without workers it maps nothing.
    setOpenRepository(repositoryAt(next, directory)?.id ?? null)
  }
  const selected = repositories.find((repository) => repository.id === openRepository)

  // A host deep link names a field by its dotted path. Open the deck level
  // that holds it first, then focus the control and put it on screen.
  const root = useRef<HTMLDivElement | null>(null)
  const honoured = useRef<string | null>(null)
  const path = focusField?.map(String).join('.') ?? null
  useEffect(() => {
    if (!path || honoured.current === path) return
    const index = path.startsWith('repositories.') ? Number(path.split('.')[1]) : Number.NaN
    if (Number.isInteger(index)) {
      const wanted = repositories[index]?.id ?? null
      if (wanted && openRepository !== wanted) {
        setOpenRepository(wanted)
        return
      }
    }
    const target = root.current?.querySelector<HTMLElement>(
      `[data-field="${CSS.escape(path)}"]`,
    )
    if (!target) return
    honoured.current = path
    target.focus()
    target.scrollIntoView({ block: 'center' })
  }, [path, openRepository])


  return (
    <div className="sentinel-ui-config" ref={root}>
      {issues.length > 0 ? (
        <StatusPanel
          variant="warn"
          headline="This will be refused as it stands"
          detail={issues.join(' · ')}
        />
      ) : null}

      <SettingsSection
        title="Investigation"
        description="The model every investigation starts with. Each run can pick another. There are no turn, token or cost caps — the session runs beside the group, you watch it, and Stop is one click away."
      >
        <SettingsList>
          <SettingsField
            field="investigation.model"
            label="Model"
            description="Picked from what the router can actually serve. Empty means every investigation must name its own, which is the safe default: a model set here spends tokens the moment somebody clicks Investigate."
            renderControl={(props) => (
              <Selector
                {...props}
                aria-label="Investigation model"
                value={selectedModel || undefined}
                groups={modelGroups(catalog, selectedModel)}
                loading={loading}
                placeholder="every investigation names its own"
                searchPlaceholder="model or provider"
                emptyMessage="The router is serving no models. Configure a provider first."
                allowEmpty
                emptyLabel="every investigation names its own"
                onClear={() => setModel('', '')}
                // A raw id stays possible: a model the catalog has not caught
                // up with is still a model the router may serve.
                onCreate={(query) => {
                  const { model, provider } = splitKey(query.trim())
                  if (model) setModel(model, provider)
                }}
                createOptionLabel={(query) => `use ${query}`}
                onChange={(next) => {
                  const { model, provider } = splitKey(next)
                  setModel(model, provider)
                }}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Repositories"
        description="Where each worker's source lives on this machine. A worker with no repository is still grouped and investigated from the trace alone."
      >
        <SettingsDeck
          open={Boolean(selected)}
          title={selected?.id ?? ''}
          description={selected?.path}
          onBack={() => setOpenRepository(null)}
          overview={
            <SettingsList>
              {repositories.length === 0 ? (
                <SettingsRow
                  label="No checkout mapped"
                  description="Without one an investigation works from the evidence alone and says so."
                />
              ) : (
                repositories.map((repository) => (
                  <SettingsRow
                    key={repository.id}
                    label={
                      <span className="sentinel-ui-repo-label">
                        <Folder size={16} aria-hidden="true" />
                        {repository.id}
                        <span className="sentinel-ui-repo-path">{repository.path}</span>
                      </span>
                    }
                    description={
                      repository.workers.length > 0 ? (
                        <span className="sentinel-ui-repo-workers">
                          {repository.workers.slice(0, SHOWN_WORKERS).map((worker) => (
                            <Chip key={worker} className="sentinel-ui-mono">
                              {worker}
                            </Chip>
                          ))}
                          {repository.workers.length > SHOWN_WORKERS ? (
                            <Chip>+ {repository.workers.length - SHOWN_WORKERS} more</Chip>
                          ) : null}
                        </span>
                      ) : (
                        <Chip tone="warning">no workers mapped to it</Chip>
                      )
                    }
                    control={
                      <Button size="sm" variant="ghost" onClick={() => setOpenRepository(repository.id)}>
                        Edit
                      </Button>
                    }
                  />
                ))
              )}
              <SettingsRow
                label={
                  unmapped.length > 0 ? (
                    <span className="sentinel-ui-quiet">
                      Unmapped workers seen in the last 7 days:{' '}
                      <span className="sentinel-ui-mono">{unmapped.join(', ')}</span>
                    </span>
                  ) : (
                    <span className="sentinel-ui-quiet">Every worker seen in the last 7 days has a repository.</span>
                  )
                }
                control={
                  // No `data-settings-deck-fallback`: `DirectoryPicker` does
                  // not forward unknown props, so it would never reach the DOM.
                  <DirectoryPicker value={null} emptyLabel="Add repository" onChange={openAt} />
                }
              />
            </SettingsList>
          }
          detail={
            selected ? (
              <SettingsList>
                <SettingsField
                  field={`repositories.${repositories.indexOf(selected)}.path`}
                  label="Folder"
                  description="The checkout an investigation reads, read-only, on this machine."
                  error={folderError}
                  renderControl={() => (
                    <DirectoryPicker
                      value={selected.path}
                      externalError={
                        facts.onDisk[selected.id] === false ? 'This folder is not on this machine any more.' : null
                      }
                      onChange={(directory) => {
                        const other = repositoryAt(config, directory)
                        if (other && other.id !== selected.id) {
                          setFolderError(`That folder is already mapped as ${other.id}.`)
                          return
                        }
                        setFolderError(null)
                        onChange(setRepositoryPath(config, selected.id, directory) as ConfigFormProps['value'])
                      }}
                    />
                  )}
                />
                <SettingsField
                  field={`repositories.${repositories.indexOf(selected)}.workers`}
                  label="Workers"
                  description="The workers whose code lives here. A worker belongs to one checkout; adding one mapped elsewhere moves it."
                  layout="stacked"
                  controlSize="full"
                  renderControl={(props) => (
                    <WorkerTokens
                      control={props}
                      label={`Workers in ${selected.id}`}
                      value={selected.workers}
                      onChange={(next) =>
                        onChange(setRepositoryWorkers(config, selected.id, next) as ConfigFormProps['value'])
                      }
                      suggestions={suggestionsFor(selected.id)}
                      empty="No worker yet — until one is added, this checkout maps nothing."
                    />
                  )}
                />
                <SettingsRow
                  label="Remove"
                  description="The mapping goes; nothing recorded changes."
                  control={
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => {
                        update(
                          'repositories',
                          repositories.filter((repository) => repository.id !== selected.id),
                        )
                        setOpenRepository(null)
                      }}
                    >
                      <Trash2 size={16} />
                      <span>Remove</span>
                    </Button>
                  }
                />
              </SettingsList>
            ) : null
          }
        />
      </SettingsSection>

      <SettingsSection
        title="Sources"
        description="What counts as an error. Both come from the engine's observability worker; nothing is installed in other workers."
      >
        <SettingsList>
          <SettingsRow
            label="Error spans"
            description="Leaf spans with status error, via the trace trigger filtered to errors."
            control={
              <Switch
                aria-label="Error spans"
                checked={Boolean(
                  (config.sources as { trace?: { enabled?: boolean } }).trace?.enabled,
                )}
                onChange={(event) => update('sources.trace.enabled', event.target.checked)}
              />
            }
          />
          <SettingsRow
            label="Error logs"
            description={`OTel ERROR records. Joined to a span occurrence when they share a trace within ${joinSeconds} s; otherwise their own group.`}
            control={
              <Switch
                aria-label="Error logs"
                checked={Boolean((config.sources as { log?: { enabled?: boolean } }).log?.enabled)}
                onChange={(event) => update('sources.log.enabled', event.target.checked)}
              />
            }
          />
          <SettingsField
            field="ignore_services"
            label="Ignored workers"
            description="Never ingested. Sentinel itself and its investigation sessions are always excluded."
            layout="stacked"
            controlSize="full"
            renderControl={(props) => (
              <WorkerTokens
                control={props}
                label="Ignored workers"
                value={ignored}
                onChange={(next) => update('ignore_services', next)}
                suggestions={facts.seen.map((worker) => ({ value: worker, description: 'failed this week' }))}
                empty="None — every worker is watched."
              />
            )}
          />
          <SettingsRow
            label="Ingest"
            description="Off stops recording entirely; nothing already recorded is lost."
            control={
              <Switch
                aria-label="Ingest"
                checked={Boolean(config.enabled)}
                onChange={(event) => update('enabled', event.target.checked)}
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Retention"
        description="Evidence is captured at ingest because the engine keeps only its last spans in memory. Counters are never dropped."
      >
        <SettingsList>
          <SettingsField
            field="retention.evidence_per_group"
            label="Full evidence per group"
            description="The first occurrence plus the most recent ones keep their span tree and logs — and one per worker version."
            renderControl={(props) => (
              <Input
                {...props}
                type="number"
                min={1}
                value={String((config.retention as { evidence_per_group?: number }).evidence_per_group ?? 5)}
                onChange={(next) => update('retention.evidence_per_group', Number(next))}
              />
            )}
          />
          <SettingsField
            field="retention.occurrences_per_group"
            label="Occurrence rows per group"
            description="Beyond this the oldest rows go; the hourly counters stay."
            renderControl={(props) => (
              <Input
                {...props}
                type="number"
                min={1}
                value={String(
                  (config.retention as { occurrences_per_group?: number }).occurrences_per_group ?? 1000,
                )}
                onChange={(next) => update('retention.occurrences_per_group', Number(next))}
              />
            )}
          />
          <SettingsField
            field="retention.resolved_ttl_days"
            label="Archive resolved groups after (days)"
            renderControl={(props) => (
              <Input
                {...props}
                type="number"
                min={1}
                value={String((config.retention as { resolved_ttl_days?: number }).resolved_ttl_days ?? 90)}
                onChange={(next) => update('retention.resolved_ttl_days', Number(next))}
              />
            )}
          />
          <SettingsField
            field="retention.cron"
            label="Prune schedule"
            description="Six or seven field cron, UTC."
            renderControl={(props) => (
              <Input
                {...props}
                value={String((config.retention as { cron?: string }).cron ?? '')}
                onChange={(next) => update('retention.cron', next)}
              />
            )}
          />
        </SettingsList>
      </SettingsSection>
    </div>
  )
}
