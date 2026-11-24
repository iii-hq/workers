import { Check, Plus, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { useWorkersChanged } from '@/hooks/use-workers-changed'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import type { ProviderListEntry } from '@/lib/models-catalog'
import { providerIconMaskUrl, providerInitial } from '@/lib/provider-icon'
import { bundledProviderMark } from '@/lib/provider-marks'
import { normalizeErrorMessage } from '@/lib/providers'
import { cn } from '@/lib/utils'
import {
  fetchRegistryProviders,
  providerIdForRegistryWorker,
  type RegistryWorker,
  registryWorkerMatchesProvider,
} from '@/lib/workers-registry'
import { fetchEngineWorkersList } from '@/pages/Workers/api/workers'
import { formatProviderLabel } from './model-picker-presentation'
import { ProviderIcon } from './ProviderIcon'

// viewport: phone chrome — the sm and md utilities here are the console's
// phone-vs-desktop presentation (touch sizes, 16px text, sheet vs popover),
// not pane layout; see viewport-breakpoint-conformance.test.ts.

/**
 * The provider's logo on a quiet square tile: the mark the running worker
 * declared, else the one bundled from this repo (a provider that is not
 * installed has declared nothing yet), else its initial.
 */
function ProviderTile({
  iconSvg,
  providerId,
  label,
}: {
  iconSvg: string | null | undefined
  providerId: string
  label: string
}) {
  return (
    <span
      aria-hidden
      className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted ring-1 ring-inset ring-border"
    >
      {providerIconMaskUrl(iconSvg) ? (
        <ProviderIcon
          iconSvg={iconSvg}
          providerId={providerId}
          label={label}
          className="size-5 sm:size-4"
        />
      ) : (
        <span className="font-inter text-sm font-semibold text-ink-faint">
          {providerInitial(label)}
        </span>
      )}
    </span>
  )
}

/** The dashed action of a search result: the Models dialog's own setup look. */
const SEARCH_ACTION_CLASS =
  'flex h-11 w-full items-center justify-center gap-2 rounded-lg border border-dashed border-border px-3 text-sm text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-40 dark:border-edge'
const SEARCH_STATUS_CLASS =
  'flex h-11 w-full items-center justify-center gap-2 rounded-lg border border-dashed border-border px-3 text-sm text-ink-faint tabular-nums dark:border-edge'

/** Base id for the browser-local `engine::on-workers-changed` handler. */
const PROVIDER_ADD_WATCH_FN = 'ade::provider-add-watch'

type AddStatus =
  | { kind: 'adding' }
  | { kind: 'done' }
  | { kind: 'failed'; error: string }

export interface AddProviderPanelProps {
  /** Providers already declared to the router; defaults to the chat context. */
  providers?: ProviderListEntry[]
  /** Deterministic registry rows for stories/tests; omitted loads the registry. */
  registryWorkers?: RegistryWorker[] | null
  /** Engine worker names already connected; omitted probes the engine. */
  installedWorkerNames?: readonly string[]
  /** Jump to a present provider's configuration once it is installed. */
  onConfigureProvider?: (providerId: string) => void
  /**
   * Search results mode (the Models dialog's search): only providers whose
   * name or worker name has every word, under an "Available to add" heading,
   * with no footnote, scroll area or empty and loading states; nothing renders
   * when nothing matches.
   */
  query?: string
  /**
   * How many registry providers match `query` (`null` while the registry
   * loads); the dialog's empty state reads it.
   */
  onMatchCount?: (count: number | null) => void
  disabled?: boolean
  className?: string
}

/**
 * Every provider worker the public registry publishes, with a one-tap add.
 * Adding runs `compose::add` — exactly what `iii call compose::add` does in
 * a terminal — and the row shows it adding until
 * the add settles. A worker added from a terminal shows as installed once it
 * connects. Once the worker registers with the router the
 * picker's provider list updates on its own; this panel only has to offer
 * the configuration shortcut.
 */
export function AddProviderPanel({
  providers,
  registryWorkers,
  installedWorkerNames,
  onConfigureProvider,
  query,
  onMatchCount,
  disabled,
  className,
}: AddProviderPanelProps) {
  const ctx = useConversationsCtxOptional()
  const live = ctx?.backend.id === 'real'
  const presentProviders = providers ?? ctx?.presentProviders ?? []
  const [registry, setRegistry] = useState<{
    workers: RegistryWorker[] | null
    error: boolean
  }>({ workers: registryWorkers ?? null, error: false })
  const [installed, setInstalled] = useState<ReadonlySet<string>>(
    () => new Set(installedWorkerNames ?? []),
  )
  const [adds, setAdds] = useState<ReadonlyMap<string, AddStatus>>(
    () => new Map(),
  )

  const loadRegistry = useCallback((signal?: AbortSignal) => {
    setRegistry({ workers: null, error: false })
    void fetchRegistryProviders({ signal })
      .then((workers) => {
        if (!signal?.aborted) setRegistry({ workers, error: false })
      })
      .catch(() => {
        if (!signal?.aborted) setRegistry({ workers: [], error: true })
      })
  }, [])

  useEffect(() => {
    if (registryWorkers !== undefined) {
      setRegistry({ workers: registryWorkers, error: false })
      return
    }
    const controller = new AbortController()
    loadRegistry(controller.signal)
    return () => controller.abort()
  }, [registryWorkers, loadRegistry])

  // Workers already connected, read again whenever one connects or leaves,
  // so a worker added from a terminal drops out of the list too.
  const refreshInstalled = useCallback(() => {
    void fetchEngineWorkersList()
      .then((list) => {
        const names = list.workers
          .map((worker) => worker.name)
          .filter((name): name is string => typeof name === 'string')
        setInstalled((current) => new Set([...current, ...names]))
      })
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    if (installedWorkerNames !== undefined) {
      setInstalled(new Set(installedWorkerNames))
      return
    }
    if (live) refreshInstalled()
  }, [installedWorkerNames, live, refreshInstalled])

  useWorkersChanged({
    enabled: live && installedWorkerNames === undefined,
    fnId: PROVIDER_ADD_WATCH_FN,
    onChange: refreshInstalled,
  })

  const addWorker = useCallback(
    (name: string) => {
      if (!live || disabled) return
      setAdds((current) => new Map(current).set(name, { kind: 'adding' }))
      void (async () => {
        const client = await getIiiClient()
        try {
          await client.call(
            'compose::add',
            { workers: [name] },
            { timeoutMs: 600_000 },
          )
          setAdds((current) => new Map(current).set(name, { kind: 'done' }))
          setInstalled((current) => new Set([...current, name]))
        } catch (err) {
          // A late timeout can reject after the add actually landed.
          try {
            const list = await fetchEngineWorkersList()
            if (list.workers.some((worker) => worker.name === name)) {
              setAdds((current) => new Map(current).set(name, { kind: 'done' }))
              setInstalled((current) => new Set([...current, name]))
              return
            }
          } catch {
            // fall through to the original error
          }
          setAdds((current) =>
            new Map(current).set(name, {
              kind: 'failed',
              error: normalizeErrorMessage(err),
            }),
          )
        }
      })()
    },
    [disabled, live],
  )

  // Providers already running are left out — this page is for what is missing.
  // A worker added from here stays listed through its lifecycle so the
  // operator sees it finish and can jump straight to its configuration.
  const rows = useMemo(() => {
    const workers = registry.workers ?? []
    return workers
      .map((worker) => {
        const present = presentProviders.find((provider) =>
          registryWorkerMatchesProvider(worker.name, provider.id),
        )
        const providerId = providerIdForRegistryWorker(worker.name)
        return {
          worker,
          present,
          providerId,
          iconSvg: present?.icon_svg ?? bundledProviderMark(providerId),
          installed: installed.has(worker.name) || present !== undefined,
          add: adds.get(worker.name),
          label:
            present?.display_name ??
            formatProviderLabel(providerId) ??
            worker.name,
        }
      })
      .filter((row) => !row.installed || row.add !== undefined)
  }, [adds, installed, presentProviders, registry.workers])
  const everythingInstalled =
    (registry.workers?.length ?? 0) > 0 && rows.length === 0
  const searchMode = query !== undefined
  const words = (query ?? '').trim().toLowerCase().split(/\s+/).filter(Boolean)
  const shownRows = searchMode
    ? rows.filter((row) => {
        const haystack = `${row.label} ${row.worker.name}`.toLowerCase()
        return (
          words.length > 0 && words.every((word) => haystack.includes(word))
        )
      })
    : rows
  const matchCount =
    registry.workers === null
      ? null
      : searchMode
        ? shownRows.length
        : rows.length
  useEffect(() => {
    onMatchCount?.(matchCount)
  }, [matchCount, onMatchCount])

  const providerList = (
    <ul
      // biome-ignore lint/a11y/noRedundantRoles: keep list semantics when CSS resets remove markers.
      role="list"
      aria-label="providers in the workers registry"
      className="flex flex-col"
    >
      {shownRows.map(
        ({ worker, present, providerId, iconSvg, installed, add, label }) => {
          const adding = add?.kind === 'adding'
          const failed = add?.kind === 'failed'
          return (
            <li
              key={worker.name}
              className="flex items-center gap-3 rounded-lg px-2 py-2.5 hover:bg-surface-hover"
            >
              <ProviderTile
                iconSvg={iconSvg}
                providerId={providerId}
                label={label}
              />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <div className="flex min-w-0 items-baseline gap-2">
                  <span className="shrink-0 font-inter text-base font-medium text-ink sm:text-sm">
                    {label}
                  </span>
                  <span className="hidden min-w-0 truncate font-mono text-xs text-ink-faint sm:inline">
                    {worker.name}
                    {worker.version ? `@${worker.version}` : ''}
                  </span>
                </div>
                {worker.description ? (
                  <p className="line-clamp-2 font-inter text-sm leading-snug text-pretty text-ink-faint sm:text-xs">
                    {worker.description}
                  </p>
                ) : null}
                {failed ? (
                  <span
                    role="alert"
                    className="font-mono text-meta text-alert break-all"
                  >
                    {add.error}
                  </span>
                ) : null}
              </div>
              <div className="flex shrink-0 items-center">
                {installed && !adding ? (
                  present && onConfigureProvider ? (
                    <Button
                      type="button"
                      variant="pill"
                      size="sm"
                      disabled={disabled}
                      onClick={() => onConfigureProvider(present.id)}
                    >
                      Configure
                    </Button>
                  ) : (
                    <span className="flex items-center gap-1.5 font-sans text-sm text-ink-faint sm:text-xs">
                      <Check className="size-4 shrink-0" aria-hidden />
                      {present ? 'Installed' : 'Starting…'}
                    </span>
                  )
                ) : adding ? (
                  <span
                    role="status"
                    className="flex items-center gap-1.5 font-sans text-sm text-ink-faint tabular-nums sm:text-xs"
                  >
                    <RefreshCw
                      className="size-4 shrink-0 animate-spin"
                      aria-hidden
                    />
                    Adding…
                  </span>
                ) : (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    disabled={disabled || !live}
                    aria-label={`${failed ? 'Retry adding' : 'Add'} ${label}`}
                    onClick={() => addWorker(worker.name)}
                    // Outlined on the popover itself: a filled card fill read as a hole
                    // in the dark popover.
                    className="bg-transparent pl-2.5"
                  >
                    {failed ? (
                      <RefreshCw aria-hidden />
                    ) : (
                      <Plus aria-hidden className="text-ink-faint" />
                    )}
                    {failed ? 'Retry' : searchMode ? 'Add provider' : 'Add'}
                  </Button>
                )}
              </div>
            </li>
          )
        },
      )}
    </ul>
  )

  if (searchMode) {
    // Each match is one more provider column in the Models dialog's grid,
    // built like a column still to configure: the provider's mark and name,
    // then one dashed action. `contents` lets the columns join that grid.
    return (
      <div className={cn('contents', className)}>
        {shownRows.map(
          ({ worker, present, providerId, iconSvg, installed, add, label }) => {
            const adding = add?.kind === 'adding'
            const failed = add?.kind === 'failed'
            return (
              <section
                key={worker.name}
                aria-label={label}
                data-registry-provider={worker.name}
                className="min-w-0"
              >
                <h3 className="mb-3 flex h-6 items-center gap-2 text-sm text-ink-faint">
                  <ProviderIcon
                    iconSvg={iconSvg}
                    label={label}
                    providerId={providerId}
                  />
                  <span className="min-w-0 truncate">{label}</span>
                  <span className="ml-auto shrink-0 text-xs">
                    Not installed
                  </span>
                </h3>
                {installed && !adding ? (
                  present && onConfigureProvider ? (
                    <button
                      type="button"
                      disabled={disabled}
                      onClick={() => onConfigureProvider(present.id)}
                      className={SEARCH_ACTION_CLASS}
                    >
                      Configure {label}
                    </button>
                  ) : (
                    <div role="status" className={SEARCH_STATUS_CLASS}>
                      <Check className="size-4 shrink-0" aria-hidden />
                      {present ? 'Installed' : 'Starting…'}
                    </div>
                  )
                ) : adding ? (
                  <div role="status" className={SEARCH_STATUS_CLASS}>
                    <RefreshCw
                      className="size-4 shrink-0 animate-spin"
                      aria-hidden
                    />
                    Adding…
                  </div>
                ) : (
                  <button
                    type="button"
                    disabled={disabled || !live}
                    aria-label={`${failed ? 'Retry' : 'Add provider'} ${label}`}
                    onClick={() => addWorker(worker.name)}
                    className={SEARCH_ACTION_CLASS}
                  >
                    {failed ? (
                      <RefreshCw className="size-4" aria-hidden />
                    ) : (
                      <Plus className="size-4" aria-hidden />
                    )}
                    {failed ? 'Retry' : 'Add provider'}
                  </button>
                )}
                {failed ? (
                  <p
                    role="alert"
                    className="mt-2 font-mono text-xs break-all text-alert"
                  >
                    {add.error}
                  </p>
                ) : null}
              </section>
            )
          },
        )}
      </div>
    )
  }

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        {registry.workers === null ? (
          <div
            role="status"
            aria-busy="true"
            aria-label="loading providers"
            className="flex flex-col"
          >
            {[0, 1, 2, 3].map((row) => (
              <div key={row} className="flex items-center gap-3 px-2 py-2.5">
                <Skeleton className="size-9 rounded-lg" />
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <Skeleton className="h-4 w-32" />
                  <Skeleton className="h-4 w-2/3" />
                </div>
                <Skeleton className="h-8 w-16" />
              </div>
            ))}
          </div>
        ) : registry.error ? (
          <div className="flex flex-col gap-3 rounded-xl bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
            <span>The workers registry is unreachable right now.</span>
            <div>
              <Button
                type="button"
                variant="pill"
                size="sm"
                onClick={() => loadRegistry()}
              >
                Retry
              </Button>
            </div>
          </div>
        ) : rows.length === 0 ? (
          <div className="rounded-xl bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
            {everythingInstalled
              ? 'Every provider in the registry is already installed.'
              : 'The registry lists no provider workers.'}
          </div>
        ) : (
          providerList
        )}
      </div>
      <p className="shrink-0 border-t border-edge px-4 py-3 font-inter text-xs leading-relaxed text-ink-faint">
        Adding a provider runs <span className="font-mono">compose::add</span> —
        the same as{' '}
        <span className="font-mono">iii call compose::add worker=…</span> in
        your terminal. New providers still need credentials: configure them once
        they appear in the list.
      </p>
    </div>
  )
}
