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
import { Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { catalogKey, modelGroups, splitKey } from './catalog.js'
import { addRepository, normalize, problems, setPath } from './form-model.js'
import { useModelCatalog } from './useModelCatalog'

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
        title="Watching"
        description="Which of the engine's signals become error groups."
      >
        <SettingsList>
          <SettingsRow
            label="Enabled"
            description="Off stops ingest entirely; nothing already recorded is lost."
            control={
              <Switch
                aria-label="Enabled"
                checked={Boolean(config.enabled)}
                onChange={(event) => update('enabled', event.target.checked)}
              />
            }
          />
          <SettingsRow
            label="Error spans"
            description="Every span the engine marks failed."
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
            description="ERROR records, joined to the span of their trace when there is one."
            control={
              <Switch
                aria-label="Error logs"
                checked={Boolean((config.sources as { log?: { enabled?: boolean } }).log?.enabled)}
                onChange={(event) => update('sources.log.enabled', event.target.checked)}
              />
            }
          />
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title="Investigation"
        description="The model an investigation opens with. There are no turn, token or cost ceilings: the session runs beside the page and Stop is one click away."
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
        description="Where each worker's code lives. An investigation can read the checkout mapped to the failing worker, and nothing else on this machine."
        action={
          <DirectoryPicker
            data-settings-deck-fallback
            value={null}
            emptyLabel="Add a checkout"
            onChange={(directory) => onChange(addRepository(config, directory) as ConfigFormProps['value'])}
          />
        }
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
                    label={repository.id}
                    description={repository.path}
                    meta={
                      repository.workers.length > 0 ? (
                        <Chip tone="neutral">{repository.workers.length} workers</Chip>
                      ) : (
                        <Chip tone="warning">no workers</Chip>
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
            </SettingsList>
          }
          detail={
            selected ? (
              <SettingsList>
                <SettingsField
                  field={`repositories.${repositories.indexOf(selected)}.workers`}
                  label="Workers"
                  description="Comma separated. A worker belongs to at most one checkout."
                  renderControl={(props) => (
                    <Input
                      {...props}
                      value={selected.workers.join(', ')}
                      onChange={(next) =>
                        update(
                          'repositories',
                          repositories.map((repository) =>
                            repository.id === selected.id
                              ? {
                                  ...repository,
                                  workers: next
                                    .split(',')
                                    .map((name: string) => name.trim())
                                    .filter(Boolean),
                                }
                              : repository,
                          ),
                        )
                      }
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
        title="Retention"
        description="Counters never shrink. What is let go is the frozen evidence behind them: the first occurrence, the most recent ones and one per worker version always stay."
      >
        <SettingsList>
          <SettingsField
            field="retention.evidence_per_group"
            label="Bundles kept per group"
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
