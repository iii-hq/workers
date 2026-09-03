import { Check, Plus, RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { z } from 'zod'
import { Button } from '@/components/ui/Button'
import { Skeleton } from '@/components/ui/Skeleton'
import { useWorkerLifecycle } from '@/hooks/use-worker-lifecycle'
import { useConversationsCtxOptional } from '@/lib/conversations-context'
import { getIiiClient } from '@/lib/iii-client'
import type { ProviderListEntry } from '@/lib/models-catalog'
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

/** Base id for the browser-local handler bound to the `worker` add trigger. */
const PROVIDER_ADD_WATCH_FN = 'console::provider-add-watch'

type AddStatus =
  | { kind: 'adding'; progress?: number }
  | { kind: 'done' }
  | { kind: 'failed'; error: string }

const workerEventSchema = z.object({
  operation: z.string().optional(),
  stage: z.string().optional(),
  worker: z.string().nullable().optional(),
  progress: z.number().nullable().optional(),
  error: z
    .object({ code: z.string().optional(), message: z.string().optional() })
    .nullable()
    .optional(),
})

export interface AddProviderPanelProps {
  /** Providers already declared to the router; defaults to the chat context. */
  providers?: ProviderListEntry[]
  /** Deterministic registry rows for stories/tests; omitted loads the registry. */
  registryWorkers?: RegistryWorker[] | null
  /** Engine worker names already connected; omitted probes the engine. */
  installedWorkerNames?: readonly string[]
  /** Jump to a present provider's configuration once it is installed. */
  onConfigureProvider?: (providerId: string) => void
  disabled?: boolean
  className?: string
}

/**
 * Every provider worker the public registry publishes, with a one-tap add.
 * Adding runs `compose::add` — exactly what `iii trigger compose::add
 * worker=<name>` does in a terminal — and the row follows the worker
 * lifecycle (downloading → done) so the operator sees the same progress the
 * harness install shows. Once the worker registers with the router the
 * picker's provider list updates on its own; this panel only has to offer
 * the configuration shortcut.
 */
export function AddProviderPanel({
  providers,
  registryWorkers,
  installedWorkerNames,
  onConfigureProvider,
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

  useEffect(() => {
    if (installedWorkerNames !== undefined) {
      setInstalled(new Set(installedWorkerNames))
      return
    }
    if (!live) return
    let cancelled = false
    void fetchEngineWorkersList()
      .then((list) => {
        if (cancelled) return
        const names = list.workers
          .map((worker) => worker.name)
          .filter((name): name is string => typeof name === 'string')
        setInstalled((current) => new Set([...current, ...names]))
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [installedWorkerNames, live])

  const handleLifecycleEvent = useCallback((payload: unknown) => {
    const parsed = workerEventSchema.safeParse(payload)
    if (!parsed.success || parsed.data.operation !== 'add') return
    const evt = parsed.data
    const worker = typeof evt.worker === 'string' ? evt.worker : ''
    if (!worker) return
    setAdds((current) => {
      const next = new Map(current)
      if (evt.stage === 'done') {
        next.set(worker, { kind: 'done' })
      } else if (evt.stage === 'failed') {
        next.set(worker, {
          kind: 'failed',
          error: evt.error?.message
            ? normalizeErrorMessage(evt.error)
            : 'failed to add the worker',
        })
      } else {
        next.set(worker, {
          kind: 'adding',
          progress: typeof evt.progress === 'number' ? evt.progress : undefined,
        })
      }
      return next
    })
    if (evt.stage === 'done') {
      setInstalled((current) => new Set([...current, worker]))
    }
  }, [])

  useWorkerLifecycle({
    enabled: live && installedWorkerNames === undefined,
    fnId: PROVIDER_ADD_WATCH_FN,
    operations: ['add'],
    onEvent: handleLifecycleEvent,
  })

  const addWorker = useCallback(
    (name: string) => {
      if (!live || disabled) return
      setAdds((current) => new Map(current).set(name, { kind: 'adding' }))
      void (async () => {
        const client = await getIiiClient()
        try {
          await client.trigger(
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
        return {
          worker,
          present,
          installed: installed.has(worker.name) || present !== undefined,
          add: adds.get(worker.name),
          label:
            present?.display_name ??
            formatProviderLabel(providerIdForRegistryWorker(worker.name)) ??
            worker.name,
        }
      })
      .filter((row) => !row.installed || row.add !== undefined)
  }, [adds, installed, presentProviders, registry.workers])
  const everythingInstalled =
    (registry.workers?.length ?? 0) > 0 && rows.length === 0

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3">
        {registry.workers === null ? (
          <div
            role="status"
            aria-busy="true"
            aria-label="loading providers"
            className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
          >
            {[0, 1, 2, 3].map((row) => (
              <div key={row} className="flex items-center gap-3 px-3 py-3">
                <Skeleton className="size-5 rounded-xs" />
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <Skeleton className="h-4 w-32" />
                  <Skeleton className="h-4 w-2/3" />
                </div>
                <Skeleton className="h-8 w-16" />
              </div>
            ))}
          </div>
        ) : registry.error ? (
          <div className="flex flex-col gap-3 rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
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
          <div className="rounded-lg bg-surface px-3 py-4 font-sans text-base text-ink-faint sm:text-sm">
            {everythingInstalled
              ? 'Every provider in the registry is already installed.'
              : 'The registry lists no provider workers.'}
          </div>
        ) : (
          <ul
            // biome-ignore lint/a11y/noRedundantRoles: keep list semantics when CSS resets remove markers.
            role="list"
            aria-label="providers in the workers registry"
            className="divide-y divide-edge overflow-hidden rounded-lg bg-surface ring-1 ring-inset ring-edge"
          >
            {rows.map(({ worker, present, installed, add, label }) => {
              const adding = add?.kind === 'adding'
              const failed = add?.kind === 'failed'
              return (
                <li
                  key={worker.name}
                  className="flex min-h-16 items-center gap-3 px-3 py-2.5"
                >
                  <ProviderIcon
                    iconSvg={present?.icon_svg}
                    label={label}
                    className="size-5 text-ink-faint sm:size-4"
                  />
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="flex min-w-0 items-baseline gap-2">
                      <span className="truncate font-sans text-base font-medium text-ink sm:text-sm">
                        {label}
                      </span>
                      <span className="hidden min-w-0 truncate font-mono text-[11px] text-ink-ghost sm:inline">
                        {worker.name}
                        {worker.version ? `@${worker.version}` : ''}
                      </span>
                    </div>
                    {worker.description ? (
                      <p className="line-clamp-2 font-sans text-sm leading-snug text-pretty text-ink-faint sm:text-[12px]">
                        {worker.description}
                      </p>
                    ) : null}
                    {failed ? (
                      <span
                        role="alert"
                        className="font-mono text-[11px] text-alert break-all"
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
                        <span className="flex items-center gap-1.5 font-sans text-sm text-ink-faint sm:text-[12px]">
                          <Check className="size-4 shrink-0" aria-hidden />
                          {present ? 'Installed' : 'Starting…'}
                        </span>
                      )
                    ) : adding ? (
                      <span
                        role="status"
                        className="flex items-center gap-1.5 font-sans text-sm text-ink-faint tabular-nums sm:text-[12px]"
                      >
                        <RefreshCw
                          className="size-4 shrink-0 animate-spin"
                          aria-hidden
                        />
                        {typeof add.progress === 'number'
                          ? `${Math.round(add.progress * 100)}%`
                          : 'Adding…'}
                      </span>
                    ) : (
                      <Button
                        type="button"
                        variant="pill"
                        size="sm"
                        disabled={disabled || !live}
                        aria-label={`${failed ? 'Retry adding' : 'Add'} ${label}`}
                        onClick={() => addWorker(worker.name)}
                      >
                        {failed ? (
                          <RefreshCw aria-hidden />
                        ) : (
                          <Plus aria-hidden />
                        )}
                        {failed ? 'Retry' : 'Add'}
                      </Button>
                    )}
                  </div>
                </li>
              )
            })}
          </ul>
        )}
      </div>
      <p className="shrink-0 px-4 pb-3 font-sans text-[12px] leading-relaxed text-ink-ghost">
        Adding a provider runs <span className="font-mono">compose::add</span> —
        the same as{' '}
        <span className="font-mono">iii trigger compose::add worker=…</span> in
        your terminal. New providers still need credentials: configure them once
        they appear in the list.
      </p>
    </div>
  )
}
