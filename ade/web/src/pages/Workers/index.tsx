import { errorMessage } from '@iii-dev/console-ui/format'
import {
  useContainerNarrow,
  usePaneState,
  useWorkerLive,
} from '@iii-dev/console-ui/hooks'
import { Boxes, ChevronLeft, Layers, Plus, X } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { useConfirm } from '@/components/ui/ConfirmDialog'
import { EmptyState } from '@/components/ui/EmptyState'
import { IconButton } from '@/components/ui/IconButton'
import { List, ListGroup, ListGroupLabel, ListItem } from '@/components/ui/List'
import { LiveRegion } from '@/components/ui/LiveRegion'
import {
  PageBody,
  PageHeader,
  PageMain,
  PageShell,
} from '@/components/ui/PageChrome'
import { PageSidebar } from '@/components/ui/PageSidebar'
import { SearchField } from '@/components/ui/SearchField'
import { Skeleton } from '@/components/ui/Skeleton'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import type { LiveAnnouncement } from '@/hooks/use-live-announcer'
import { getDefaultBackend } from '@/lib/backend'
import { getIiiClient, type IiiClient } from '@/lib/iii-client'
import type {
  ExtensionIii,
  PageCommandsApi,
  PanelSide,
} from '@/types/injectable-ui'
import { AddWorkerDialog } from './add-worker'
import {
  type Accepted,
  composeApi,
  type MutationOutcome,
  type OperationSnapshot,
  type Snapshot,
} from './compose-api'
import { ContainerView, WorkerView } from './container'
import { followOperation } from './follow-operation'
import { groupContainers, toneFor } from './model'
import { takePendingWorkerSearch } from './pending-selection'
import { ProjectView } from './project'
import './workers.css'

interface WorkersProps {
  /** Close the hosting pane — the header's standard ✕ when present. */
  onRequestClose?: () => void
  /** The pane's command registrar: the page's verbs and keys. */
  commands?: PageCommandsApi
  panelSide?: PanelSide
  tabId?: string
  paneId?: string
  /** Report unsaved settings so closing the pane asks first. */
  setDirty?: (dirty: boolean | string) => void
}

const PROJECT = 'project'
const SKELETON_ROWS = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']

export type Activity = {
  label: string
  state: 'running' | 'succeeded' | 'failed'
  operationId?: string
  snapshot?: OperationSnapshot
  detail?: string
}

/** Everything a view needs to change the project and follow the result. */
export type Actions = {
  busy: boolean
  /** A daemon operation that answers once it is done (up, down, restart). */
  run: (label: string, call: () => Promise<MutationOutcome>) => Promise<boolean>
  /**
   * An operation the daemon accepts and runs in the background (add, update,
   * remove, edit), followed by its pushed progress. `start` must submit it
   * under `operationId`: its progress is bound before it is submitted.
   */
  track: (
    label: string,
    start: (operationId: string) => Promise<Accepted>,
  ) => Promise<boolean>
  confirm: ReturnType<typeof useConfirm>['confirm']
  /** Declared containers that are stopped or failed: an add or remove starts them. */
  idle: string[]
}

function progressDetail(
  snapshot: OperationSnapshot | undefined,
): string | undefined {
  if (!snapshot) return 'Waiting for the daemon to accept it'
  const detail = snapshot.last_event?.detail ?? snapshot.phase
  return snapshot.total > 1
    ? `${detail} · ${snapshot.completed} of ${snapshot.total}`
    : detail
}

/** The console's Workers screen: every worker the compose daemon runs or the engine sees. */
export function Workers(props: WorkersProps) {
  const real = getDefaultBackend().id === 'real'
  const [client, setClient] = useState<IiiClient | null>(null)
  useEffect(() => {
    if (!real) return
    let cancelled = false
    void getIiiClient().then((next) => !cancelled && setClient(next))
    return () => {
      cancelled = true
    }
  }, [real])
  if (!real) {
    return (
      <PageShell aria-label="workers">
        <PageHeader
          icon={<Layers />}
          title="Workers"
          onClose={props.onRequestClose}
        />
        <EmptyState
          icon={Boxes}
          title="No live workers here"
          description="Workers are listed when the console is connected to an engine."
        />
      </PageShell>
    )
  }
  if (!client) {
    return (
      <PageShell aria-label="workers">
        <PageHeader
          icon={<Layers />}
          title="Workers"
          onClose={props.onRequestClose}
        />
        <div
          className="wk-skeletons"
          role="status"
          aria-busy="true"
          aria-label="Connecting"
        >
          <Skeleton className="wk-skeleton-block" />
        </div>
      </PageShell>
    )
  }
  return <WorkersPage iii={client} {...props} />
}

function WorkersPage({
  iii,
  onRequestClose,
  panelSide = 'left',
  paneId,
  tabId,
  commands,
  setDirty,
}: WorkersProps & { iii: ExtensionIii }) {
  const fileRef = useRef<string | undefined>(undefined)
  const api = useMemo(() => composeApi(iii, () => fileRef.current), [iii])
  const live = useWorkerLive<Snapshot>({
    iii,
    triggers: ['console::compose::changed'],
    handlerId: 'iii::console::workers::changed',
    fetch: api.snapshot,
  })
  const data = live.data
  if (data?.status.file) fileRef.current = data.status.file

  const [selection, setSelection] = usePaneState<string>(
    `workers:selection:${paneId || tabId}`,
    PROJECT,
  )
  const [drilled, setDrilled] = useState(false)
  const [query, setQuery] = useState('')
  const [adding, setAdding] = useState(false)
  const [activity, setActivity] = useState<Activity | null>(null)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(
    null,
  )
  const { confirm, dialog } = useConfirm()
  const { ref: bodyRef, narrow } = useContainerNarrow()
  const filterRef = useRef<HTMLInputElement>(null)
  const alive = useRef(true)
  // Operations this page follows: leaving it unbinds their progress.
  const following = useRef(new Set<AbortController>())
  useEffect(() => {
    alive.current = true
    const followed = following.current
    return () => {
      alive.current = false
      for (const controller of followed) controller.abort()
      followed.clear()
    }
  }, [])

  const announce = useCallback(
    (text: string, urgency: 'polite' | 'assertive' = 'polite') => {
      setAnnouncement((prev) => ({ seq: (prev?.seq ?? 0) + 1, text, urgency }))
    },
    [],
  )

  const select = useCallback(
    (name: string) => {
      setSelection(name)
      setDrilled(true)
    },
    [setSelection],
  )

  const finish = useCallback(
    (next: Activity) => {
      if (!alive.current) return
      setActivity(next)
      announce(
        next.state === 'failed' ? `${next.label} failed` : `${next.label} done`,
        next.state === 'failed' ? 'assertive' : 'polite',
      )
      live.refresh()
    },
    [announce, live],
  )

  const busy = activity?.state === 'running'
  const idle = useMemo(
    () =>
      (data?.status.containers ?? [])
        .filter((c) => c.state === 'stopped' || c.state === 'failed')
        .map((c) => c.container),
    [data?.status.containers],
  )

  const actions: Actions = useMemo(
    () => ({
      busy,
      confirm,
      idle,
      async run(label, call) {
        setActivity({ label, state: 'running' })
        try {
          const outcome = await call()
          const failed = outcome.status === 'failed'
          finish({
            label,
            state: failed ? 'failed' : 'succeeded',
            detail: outcome.error?.message,
          })
          return !failed
        } catch (cause) {
          finish({ label, state: 'failed', detail: errorMessage(cause) })
          return false
        }
      },
      async track(label, start) {
        setActivity({ label, state: 'running' })
        const controller = new AbortController()
        following.current.add(controller)
        try {
          // Pushed progress, bound before the mutation is submitted.
          const snapshot = await followOperation({
            iii,
            start,
            read: api.operation,
            signal: controller.signal,
            onProgress: (progress, operationId) => {
              if (alive.current)
                setActivity({
                  label,
                  state: 'running',
                  operationId,
                  snapshot: progress,
                })
            },
          })
          if (snapshot === null || !alive.current) return false
          const ok = snapshot.status === 'succeeded'
          finish({
            label:
              snapshot.status === 'cancelled' ? `${label} (cancelled)` : label,
            state: ok ? 'succeeded' : 'failed',
            operationId: snapshot.operation_id,
            snapshot,
            detail: ok ? undefined : snapshot.last_event?.detail,
          })
          return ok
        } catch (cause) {
          finish({ label, state: 'failed', detail: errorMessage(cause) })
          return false
        } finally {
          following.current.delete(controller)
        }
      },
    }),
    [api, busy, confirm, finish, idle, iii],
  )

  const declared = useMemo(
    () => new Map((data?.project?.containers ?? []).map((c) => [c.name, c])),
    [data],
  )
  const containers = data?.status.containers ?? []
  const workers = data?.workers ?? []
  const groups = useMemo(
    () => groupContainers(containers, declared, query, workers),
    [containers, declared, query, workers],
  )
  const selected =
    selection === PROJECT
      ? null
      : (containers.find((c) => c.container === selection) ?? null)
  const outside =
    selection === PROJECT || selected
      ? null
      : (workers.find(
          (w) => w.name === selection && w.managementKind !== 'compose',
        ) ?? null)
  const ready = containers.filter((c) => c.state === 'ready').length
  const failing = containers.filter((c) => c.state === 'failed').length

  // A worker that went away (removed, disconnected, another file) falls back
  // to the project. One this page has not seen yet (just added, the snapshot
  // still on its way) keeps the selection; so does nothing on the first
  // snapshot, which only drops a selection remembered from an earlier visit.
  const seen = useRef<Set<string> | null>(null)
  useEffect(() => {
    if (!data) return
    const present = new Set([
      ...containers.map((c) => c.container),
      ...workers.map((w) => w.name),
    ])
    const before = seen.current
    seen.current = present
    if (selection === PROJECT || present.has(selection)) return
    if (before === null || before.has(selection)) setSelection(PROJECT)
  }, [data, containers, workers, selection, setSelection])

  // The palette opens this screen on the worker it picked: select it when it
  // is here, otherwise filter the list by what was asked for.
  const [pending, setPending] = useState(takePendingWorkerSearch)
  useEffect(() => {
    if (!pending || !data) return
    setPending(null)
    if (
      data.workers.some((w) => w.name === pending) ||
      containers.some((c) => c.container === pending)
    )
      select(pending)
    else setQuery(pending)
  }, [pending, data, containers, select])

  useEffect(
    () =>
      commands?.register([
        {
          id: 'refresh',
          title: 'Refresh workers',
          detail: 'Read the compose project and the engine again',
          keywords: ['reload', 'fleet'],
          run: live.refresh,
        },
        {
          id: 'add',
          title: 'Add worker…',
          detail: 'Declare one from the registry or a local directory',
          keywords: ['compose', 'install', 'registry'],
          run: () => setAdding(true),
        },
        {
          id: 'project',
          title: 'Compose project',
          detail: 'Start order, package updates, compose file and daemon',
          keywords: ['compose', 'daemon', 'updates'],
          run: () => select(PROJECT),
        },
        {
          id: 'filter',
          title: 'Filter workers',
          keywords: ['search', 'find'],
          run: () => {
            setDrilled(false)
            window.requestAnimationFrame(() => filterRef.current?.focus())
          },
        },
      ]),
    [commands, live.refresh, select],
  )

  const latest = useLatest(api, data)
  const engineWorkers = useMemo(
    () =>
      new Set(
        (data?.workers ?? [])
          .filter((w) => w.managementKind === 'internal')
          .map((w) => w.name),
      ),
    [data?.workers],
  )
  const updates = containers.filter((c) => latest.outdated(c.container)).length
  const showMain = !narrow || drilled
  const showSide = !narrow || !drilled

  const sidebar = (
    <PageSidebar
      label="Workers"
      side={panelSide}
      narrow={narrow}
      collapsible
      resizable
      storageKey="workers:list"
      defaultWidth={300}
      minWidth={220}
      maxWidth={420}
    >
      <div className="wk-side">
        <SearchField
          ref={filterRef}
          aria-label="Filter workers"
          placeholder="Filter workers"
          value={query}
          onChange={setQuery}
        />
        {!data ? (
          <div
            className="wk-skeletons"
            role="status"
            aria-busy="true"
            aria-label="Loading workers"
          >
            {SKELETON_ROWS.map((id) => (
              <Skeleton key={id} className="wk-skeleton-row" />
            ))}
          </div>
        ) : (
          <List aria-label="Project and workers">
            {data.composed ? (
              <ListItem
                selected={selection === PROJECT}
                aria-current={selection === PROJECT ? 'page' : undefined}
                leading={<Layers />}
                label="Project"
                description={[
                  `${ready} of ${containers.length} ready`,
                  updates
                    ? `${updates} update${updates === 1 ? '' : 's'}`
                    : null,
                ]
                  .filter(Boolean)
                  .join(' · ')}
                trailing={
                  failing ? (
                    <Badge variant="alert">{failing} failed</Badge>
                  ) : undefined
                }
                onClick={() => select(PROJECT)}
              />
            ) : null}
            {groups.map((group) => (
              <ListGroup key={group.id}>
                <ListGroupLabel>{group.label}</ListGroupLabel>
                {group.items.map((item) => {
                  const tone = toneFor(item.state)
                  const update = latest.outdated(item.name)
                  return (
                    <ListItem
                      key={item.name}
                      selected={selection === item.name}
                      aria-current={
                        selection === item.name ? 'page' : undefined
                      }
                      leading={
                        <StatusDot
                          tone={tone.dot}
                          pulse={item.state === 'starting'}
                          aria-label={item.state}
                        />
                      }
                      label={<span className="wk-mono">{item.name}</span>}
                      description={
                        <span
                          className={item.failed ? 'wk-alert-text' : undefined}
                        >
                          {item.detail}
                        </span>
                      }
                      trailing={
                        update ? (
                          <Badge
                            variant="accent"
                            aria-label={`Update available: ${update}`}
                          >
                            {update}
                          </Badge>
                        ) : undefined
                      }
                      onClick={() => select(item.name)}
                    />
                  )
                })}
              </ListGroup>
            ))}
            {query && groups.length === 0 ? (
              <EmptyState
                compact
                icon={Boxes}
                title="No containers match"
                description={`Nothing is called “${query}”.`}
                action={{ label: 'Clear filter', onClick: () => setQuery('') }}
              />
            ) : null}
          </List>
        )}
      </div>
    </PageSidebar>
  )

  return (
    <PageShell className="wk-shell" aria-label="workers">
      <PageHeader
        icon={<Layers />}
        title="Workers"
        description={
          data?.status.namespace ? (
            <span className="wk-mono">{data.status.namespace}</span>
          ) : undefined
        }
        actions={
          <>
            <span className="wk-live" role="status">
              <StatusDot
                tone={live.error ? 'warn' : 'accent'}
                pulse={live.live}
                aria-hidden
              />
              {live.error
                ? 'daemon unreachable'
                : live.live
                  ? 'live'
                  : 'polling'}
            </span>
            <Button variant="ghost" size="sm" onClick={() => setAdding(true)}>
              <Plus />
              Add worker
            </Button>
          </>
        }
        onClose={onRequestClose}
      />
      <div ref={bodyRef} className="wk-body">
        <PageBody side={panelSide}>
          {showSide ? sidebar : null}
          {showMain ? (
            <PageMain className="wk-main">
              {narrow ? (
                <Button
                  variant="ghost"
                  size="sm"
                  className="wk-back"
                  onClick={() => setDrilled(false)}
                >
                  <ChevronLeft />
                  Workers
                </Button>
              ) : null}
              {data && !data.composed ? (
                <StatusPanel
                  variant="info"
                  headline="No compose daemon answered"
                  detail="Only the workers connected to the engine are listed. Start one with iii compose in the project directory."
                />
              ) : null}
              {live.error ? (
                <StatusPanel
                  variant="warn"
                  headline="Workers are unavailable"
                  detail={live.error}
                  action={
                    <Button variant="pill" size="sm" onClick={live.refresh}>
                      Retry
                    </Button>
                  }
                />
              ) : null}
              {activity ? (
                <StatusPanel
                  variant={
                    activity.state === 'running'
                      ? 'info'
                      : activity.state === 'failed'
                        ? 'alert'
                        : 'success'
                  }
                  headline={
                    activity.state === 'running'
                      ? `${activity.label}…`
                      : activity.state === 'failed'
                        ? `${activity.label} failed`
                        : `${activity.label} done`
                  }
                  detail={
                    activity.state === 'running'
                      ? progressDetail(activity.snapshot)
                      : activity.detail
                  }
                  action={
                    activity.state === 'running' ? (
                      activity.operationId ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            void api.cancel(activity.operationId as string)
                          }
                        >
                          Cancel
                        </Button>
                      ) : undefined
                    ) : (
                      <IconButton
                        label="Dismiss"
                        variant="ghost"
                        onClick={() => setActivity(null)}
                      >
                        <X />
                      </IconButton>
                    )
                  }
                />
              ) : null}
              {!data ? (
                <div
                  className="wk-skeletons"
                  role="status"
                  aria-busy="true"
                  aria-label="Loading"
                >
                  <Skeleton className="wk-skeleton-title" />
                  <Skeleton className="wk-skeleton-block" />
                </div>
              ) : selected ? (
                <ContainerView
                  key={selected.container}
                  api={api}
                  actions={actions}
                  container={selected}
                  declared={declared.get(selected.container) ?? null}
                  project={data.project}
                  status={data.status}
                  latest={latest.of(selected.container)}
                  onSelect={select}
                  onRemoved={() => select(PROJECT)}
                  setDirty={setDirty}
                  connected={workers.some(
                    (w) =>
                      w.name === selected.container && w.status === 'connected',
                  )}
                />
              ) : outside ? (
                <WorkerView
                  key={outside.name}
                  worker={outside}
                  actions={actions}
                  onStopped={live.refresh}
                />
              ) : data.composed ? (
                <ProjectView
                  api={api}
                  actions={actions}
                  snapshot={data}
                  latest={latest}
                  onSelect={select}
                />
              ) : (
                <EmptyState
                  icon={Boxes}
                  title="Pick a worker"
                  description="Its functions and trigger types show here."
                />
              )}
            </PageMain>
          ) : null}
        </PageBody>
      </div>
      <AddWorkerDialog
        open={adding}
        onOpenChange={setAdding}
        api={api}
        actions={actions}
        project={data?.project ?? null}
        engine={engineWorkers}
        onAdded={(name) => select(name)}
      />
      {dialog}
      <LiveRegion announcement={announcement} />
    </PageShell>
  )
}

export type Latest = {
  /** The registry's latest version for a package container, once known. */
  of: (container: string) => string | null
  /** The latest version when the container pins an older exact one. */
  outdated: (container: string) => string | null
}

/** Registry `latest` for every declared package, fetched once per set of packages. */
function useLatest(
  api: ReturnType<typeof composeApi>,
  data: Snapshot | null,
): Latest {
  const packages = (data?.project?.containers ?? []).filter(
    (c) => c.source === 'package',
  )
  const key = packages.map((c) => c.name).join(',')
  const [versions, setVersions] = useState<Record<string, string>>({})
  useEffect(() => {
    if (!key) return
    let cancelled = false
    void Promise.allSettled(
      key.split(',').map((name) => api.versions(name)),
    ).then((results) => {
      if (cancelled) return
      const next: Record<string, string> = {}
      for (const result of results) {
        if (result.status !== 'fulfilled') continue
        const found =
          result.value.versions.find((v) => v.tags.includes('latest')) ??
          result.value.versions[0]
        if (found) next[result.value.container] = found.version
      }
      setVersions(next)
    })
    return () => {
      cancelled = true
    }
  }, [api, key])
  const declared = new Map(packages.map((c) => [c.name, c.version]))
  return {
    of: (container) => versions[container] ?? null,
    outdated: (container) => {
      const pinned = declared.get(container)
      const newest = versions[container]
      return pinned && newest && /^\d/.test(pinned) && pinned !== newest
        ? newest
        : null
    },
  }
}
