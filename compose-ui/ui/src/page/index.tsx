import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Chip,
  ConfirmDialog,
  EmptyState,
  type Host,
  IconButton,
  Input,
  List,
  ListItem,
  PageBody,
  PageHeader,
  PageMain,
  type PageRenderProps,
  PageShell,
  PageSidebar,
  Skeleton,
  StatusDot,
  StatusPanel,
  Table,
  TableBody,
  TableCell,
  TableFrame,
  TableHead,
  TableHeader,
  TableRow,
  TableViewport,
  TerminalStream,
  uiClasses,
} from '@iii-dev/console-ui'
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  Boxes,
  FileText,
  Layers,
  Package,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Server,
  ShieldCheck,
  Square,
  X,
} from './icons'
import { Topology } from './Topology'
import type { TopologyInput } from './topology-layout'

type Props = PageRenderProps & { host: Host }

type ContainerState = 'starting' | 'ready' | 'failed' | 'stopped'

type Container = {
  container: string
  state: ContainerState | string
  owned?: boolean
  pid?: number | null
  last_error?: string | null
}

type Status = {
  namespace?: string
  file?: string
  state_dir?: string
  daemon_pid?: number
  containers?: Container[]
}

type ListeningPort = { port: number; address: string }
type DeclaredContainer = {
  name: string
  source: 'path' | 'package' | 'unknown'
  ref: string
  version: string | null
  start_after: string[]
  environment: string[]
  run: string | null
  pid: number | null
  ports: ListeningPort[]
}
type Project = {
  file: string
  namespace: string | null
  engine_url: string | null
  engine_host: string | null
  engine_port: number | null
  startup_timeout: string | null
  stop_timeout: string | null
  daemon_pid: number | null
  daemon_ports: ListeningPort[]
  containers: DeclaredContainer[]
}
type ProjectSummary = { namespace?: string; file?: string; containers?: Container[] }
type ProjectList = { daemon?: string; daemon_namespace?: string; daemon_pid?: number; projects?: ProjectSummary[] }

type ContainerResult = { container: string; changed?: boolean; state?: string; error?: unknown }
type OpResult = { changed?: boolean; operation_id?: string; status?: string; containers?: ContainerResult[] }
type ReconcileResult = {
  changed?: boolean
  container?: string | null
  declared?: string[]
  workers?: string[]
  detail?: string | null
  status?: string
  from?: string | null
  to?: string | null
  version?: string | null
  up?: OpResult | null
  down?: OpResult | null
  restarted?: OpResult | null
}
type Validation = { namespace?: string; start_order?: string[]; deferred_packages?: string[] }
type LogTail = { container: string; path: string; lines: string[]; size: number; truncated: boolean; missing: boolean }
type ChangedEvent = { kind?: string; file?: string; namespace?: string; state_dir?: string; path?: string }

type Section = 'topology' | 'overview' | 'containers' | 'workers' | 'daemon'

type Confirm = { title: string; description: string; confirmLabel: string; run: () => void }

const EVENTS_FN = 'iii::compose-ui::changed'
const LOG_LINES = 200

const sections: {
  id: Section
  label: string
  description: string
  icon: (props: { className?: string }) => ReactNode
}[] = [
  { id: 'topology', label: 'Topology', description: 'Engine, namespace, dependencies', icon: Layers },
  { id: 'overview', label: 'Overview', description: 'Project health, lifecycle', icon: Activity },
  { id: 'containers', label: 'Containers', description: 'State, PIDs, logs', icon: Boxes },
  { id: 'workers', label: 'Workers', description: 'Add, update, remove packages', icon: Package },
  { id: 'daemon', label: 'Daemon', description: 'Projects and supervisor', icon: Server },
]

function describe(cause: unknown): string {
  const text = (() => {
    if (cause instanceof Error) return cause.message
    if (cause && typeof cause === 'object') {
      const message = (cause as { message?: unknown }).message
      if (typeof message === 'string') return message
      try {
        return JSON.stringify(cause)
      } catch {
        return String(cause)
      }
    }
    return String(cause)
  })()
  return text.replace(/^handler error:\s*/i, '')
}

type StateStyle = { dot: 'accent' | 'alert' | 'warn' | 'ink'; badge: 'ok' | 'warn' | 'alert' | 'default' }

const STATE_STYLE: Record<ContainerState, StateStyle> = {
  ready: { dot: 'accent', badge: 'ok' },
  starting: { dot: 'warn', badge: 'warn' },
  failed: { dot: 'alert', badge: 'alert' },
  stopped: { dot: 'ink', badge: 'default' },
}

function styleFor(state: string): StateStyle {
  return STATE_STYLE[state as ContainerState] ?? STATE_STYLE.stopped
}

function running(state: string) {
  return state === 'ready' || state === 'starting'
}

function supervision(c: Container) {
  if (c.owned) return 'managed'
  return running(c.state) ? 'external' : '–'
}

function changedCount(result: OpResult | null | undefined): number {
  return (result?.containers ?? []).filter((c) => c.changed).length
}

function opSummary(verb: string, result: OpResult | null | undefined): string {
  const count = changedCount(result)
  if (result?.status === 'failed') return `${verb} failed`
  return count === 0 ? `${verb}: nothing to change` : `${verb}: ${count} container${count === 1 ? '' : 's'} changed`
}

function opErrors(result: OpResult | null | undefined): string | null {
  const failed = (result?.containers ?? []).filter((c) => c.error)
  if (failed.length === 0) return null
  return failed.map((c) => `${c.container}: ${describe(c.error)}`).join('\n')
}

function reconcileSummary(verb: string, result: ReconcileResult): string {
  const parts = [result.detail?.trim()].filter(Boolean) as string[]
  if (result.from || result.to) parts.push(`${result.from ?? '?'} → ${result.to ?? result.version ?? 'latest'}`)
  const touched = changedCount(result.up) + changedCount(result.down) + changedCount(result.restarted)
  if (touched) parts.push(`${touched} container${touched === 1 ? '' : 's'} changed`)
  return parts.length ? `${verb}: ${parts.join(' · ')}` : `${verb} done`
}

type Health = { tone: 'success' | 'warning' | 'danger' | 'neutral'; label: string }

function describeHealth(counts: Record<ContainerState, number>, total: number): Health {
  if (counts.failed) return { tone: 'danger', label: `${counts.failed} failing` }
  if (counts.starting) return { tone: 'warning', label: `${counts.starting} starting` }
  if (counts.stopped) return { tone: 'neutral', label: `${counts.stopped} stopped` }
  if (total) return { tone: 'success', label: 'all ready' }
  return { tone: 'neutral', label: 'no containers' }
}

function shortRef(ref: string): string {
  const parts = ref.split('/').filter(Boolean)
  if (parts.length <= 2) return ref
  return `…/${parts.slice(-2).join('/')}`
}

function sourceCell(declared: DeclaredContainer | undefined): ReactNode {
  if (!declared) return <span className="cu-faint">–</span>
  if (declared.source === 'package') {
    return (
      <span className="cu-mono cu-source" title={declared.ref}>
        {declared.ref.split('/').pop()}
        {declared.version ? `@${declared.version}` : ''}
      </span>
    )
  }
  if (declared.source === 'path') {
    return (
      <span className="cu-mono cu-source" title={declared.ref}>
        path {shortRef(declared.ref)}
      </span>
    )
  }
  return (
    <span className="cu-mono cu-source" title={declared.ref}>
      {declared.ref || '–'}
    </span>
  )
}

function portsCell(declared: DeclaredContainer | undefined): ReactNode {
  if (!declared || declared.ports.length === 0) return <span className="cu-faint">–</span>
  return declared.ports.map((p) => `${p.address === '*' ? '' : `${p.address}:`}${p.port}`).join(', ')
}

function portSummary(project: Project | null): ReactNode {
  if (!project) return <span className="cu-faint">–</span>
  const rows = project.containers.filter((c) => c.ports.length > 0)
  if (rows.length === 0) return <span className="cu-faint">no container listens on TCP</span>
  return (
    <span className="cu-chips">
      {rows.map((c) => (
        <Chip key={c.name} tone="neutral" title={`${c.name} pid ${c.pid ?? '–'}`}>
          <span className="cu-mono">
            {c.name} {c.ports.map((p) => p.port).join(', ')}
          </span>
        </Chip>
      ))}
    </span>
  )
}

function NodeChip({ name, onPick }: { name: string; onPick: (name: string) => void }) {
  return (
    <Chip
      tone="neutral"
      role="button"
      tabIndex={0}
      onClick={() => onPick(name)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onPick(name)
        }
      }}
    >
      <span className="cu-mono">{name}</span>
    </Chip>
  )
}

function Field({ id, label, hint, children }: { id: string; label: string; hint?: string; children: ReactNode }) {
  return (
    <div className={uiClasses.field}>
      <label htmlFor={id} className={uiClasses.fieldLabel}>
        {label}
      </label>
      {children}
      {hint ? <span className={uiClasses.fieldDescription}>{hint}</span> : null}
    </div>
  )
}

function SectionCard({ title, actions, children }: { title: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <Card className="cu-card">
      <CardHeader className="cu-card-header">
        <span className="cu-card-title">{title}</span>
        {actions}
      </CardHeader>
      <CardBody className="cu-card-body">{children}</CardBody>
    </Card>
  )
}

function Facts({ items }: { items: { term: string; children: ReactNode; wide?: boolean }[] }) {
  return (
    <dl className="cu-facts">
      {items.map((item) => (
        <div key={item.term} className={item.wide ? 'cu-facts-wide' : undefined}>
          <dt>{item.term}</dt>
          <dd>{item.children}</dd>
        </div>
      ))}
    </dl>
  )
}

function LoadingRows({ rows = 3 }: { rows?: number }) {
  return (
    <div className="cu-skeletons" role="status" aria-busy="true" aria-label="Loading">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="cu-skeleton" />
      ))}
    </div>
  )
}

function Stat({ value, label, tone }: { value: ReactNode; label: string; tone?: 'alert' | 'warn' }) {
  return (
    <div className="cu-stat" data-tone={tone}>
      <span className="cu-stat-value">{value}</span>
      <span className="cu-stat-label">{label}</span>
    </div>
  )
}

export function ComposePage({ host, onRequestClose, panelSide, commands, panelContext }: Props) {
  const [section, setSection] = useState<Section>('topology')
  const [selectedNode, setSelectedNode] = useState<string | null>(null)
  const [status, setStatus] = useState<Status | null>(null)
  const [projects, setProjects] = useState<ProjectList | null>(null)
  const [project, setProject] = useState<Project | null>(null)
  const [loaded, setLoaded] = useState(false)
  const [unavailable, setUnavailable] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [busy, setBusy] = useState<string | null>(null)
  const [feedback, setFeedback] = useState<{ kind: 'error' | 'success'; text: string } | null>(null)
  const [query, setQuery] = useState('')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [logs, setLogs] = useState<Record<string, LogTail | { error: string } | undefined>>({})
  const [validation, setValidation] = useState<Validation | null>(null)
  const [workerInput, setWorkerInput] = useState('')
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const generation = useRef(0)
  const filterRef = useRef<HTMLInputElement>(null)
  const workerRef = useRef<HTMLInputElement>(null)

  const trigger = useCallback(
    <T,>(fn: string, payload: Record<string, unknown> = {}, timeoutMs = 60_000) =>
      host.iii.trigger<T>(fn, payload, { timeoutMs }),
    [host],
  )

  const fileRef = useRef<string | undefined>(undefined)
  const expandedRef = useRef<string | null>(null)
  expandedRef.current = expanded

  const refresh = useCallback(
    async (visible = true) => {
      const mine = ++generation.current
      if (visible) setRefreshing(true)
      try {
        const file = fileRef.current
        const [nextStatus, nextProjects, nextProject] = await Promise.all([
          trigger<Status>('compose::status', file ? { file } : {}, 10_000),
          trigger<ProjectList>('compose::list', {}, 10_000).catch(() => null),
          trigger<Project>('compose-ui::project', file ? { file } : {}, 15_000).catch(() => null),
        ])
        if (mine !== generation.current) return
        fileRef.current = nextStatus.file ?? fileRef.current
        setStatus(nextStatus)
        setProjects(nextProjects)
        setProject(nextProject)
        setUnavailable(null)
        setUpdatedAt(Date.now())
      } catch (cause) {
        if (mine !== generation.current) return
        setUnavailable(describe(cause))
      } finally {
        if (mine === generation.current) {
          setLoaded(true)
          if (visible) setRefreshing(false)
        }
      }
    },
    [trigger],
  )

  const loadLogs = useCallback(
    async (container: string) => {
      try {
        const tail = await trigger<LogTail>('compose-ui::logs', { container, lines: LOG_LINES }, 15_000)
        setLogs((prev) => ({ ...prev, [container]: tail }))
      } catch (cause) {
        setLogs((prev) => ({ ...prev, [container]: { error: describe(cause) } }))
      }
    },
    [trigger],
  )

  useEffect(() => {
    void refresh()
  }, [refresh])

  useEffect(() => {
    let timer: number | null = null
    const offHandler = host.iii.on<ChangedEvent>(EVENTS_FN, () => {
      if (timer !== null) window.clearTimeout(timer)
      timer = window.setTimeout(() => {
        void refresh(false)
        if (expandedRef.current) void loadLogs(expandedRef.current)
      }, 80)
    })
    const offTrigger = host.iii.registerTrigger({
      type: 'compose-ui::changed',
      function_id: `${EVENTS_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      if (timer !== null) window.clearTimeout(timer)
      offTrigger()
      offHandler()
    }
  }, [host, refresh, loadLogs])

  useEffect(() => {
    const context = panelContext?.context
    const container =
      context && typeof context === 'object' && 'container' in context
        ? (context as { container?: unknown }).container
        : null
    if (typeof container !== 'string' || !container) return
    setSection('topology')
    setSelectedNode(container)
  }, [panelContext?.id, panelContext?.context])

  const act = useCallback(
    async <T,>(key: string, run: () => Promise<T>, summarize: (result: T) => string) => {
      setBusy(key)
      setFeedback(null)
      try {
        const result = await run()
        setFeedback({ kind: 'success', text: summarize(result) })
        await refresh(false)
        return result
      } catch (cause) {
        setFeedback({ kind: 'error', text: describe(cause) })
        return null
      } finally {
        setBusy(null)
        setConfirm(null)
      }
    },
    [refresh],
  )

  const withFile = useCallback((payload: Record<string, unknown>) => {
    const file = fileRef.current
    return file ? { file, ...payload } : payload
  }, [])

  const lifecycle = useCallback(
    (verb: 'up' | 'down' | 'restart', container?: string) => {
      const label = { up: 'Start', down: 'Stop', restart: 'Restart' }[verb]
      const target = container ?? 'project'
      return act(
        `${verb}:${target}`,
        () => trigger<OpResult>(`compose::${verb}`, withFile(container ? { container } : {})),
        (r) => {
          const errors = opErrors(r)
          if (errors) throw new Error(errors)
          return opSummary(`${label} ${target}`, r)
        },
      )
    },
    [act, trigger, withFile],
  )

  const confirmLifecycle = (verb: 'down' | 'restart', container?: string) => {
    const target = container ?? 'the whole project'
    setConfirm({
      title: verb === 'down' ? `Stop ${target}?` : `Restart ${target}?`,
      description:
        verb === 'down'
          ? container
            ? 'Compose stops this container and everything that depends on it, in reverse dependency order.'
            : 'Every container stops in reverse dependency order. The daemon keeps running.'
          : container
            ? 'Compose restarts this container in place without touching its dependency graph.'
            : 'Every container restarts in dependency order.',
      confirmLabel: verb === 'down' ? 'Stop' : 'Restart',
      run: () => void lifecycle(verb, container),
    })
  }

  const removeWorker = (worker: string) =>
    setConfirm({
      title: `Remove ${worker} from the compose file?`,
      description:
        'The declaration and every dependency reference to it leave worker-compose.yaml; only that worker stops.',
      confirmLabel: 'Remove worker',
      run: () =>
        void act(
          `remove:${worker}`,
          () => trigger<ReconcileResult>('compose::remove', withFile({ worker })),
          (r) => reconcileSummary(`Removed ${worker}`, r),
        ),
    })

  const updateWorker = (worker: string) =>
    setConfirm({
      title: `Update ${worker}?`,
      description: 'Compose moves the declared package to the requested or latest version, then restarts the project.',
      confirmLabel: 'Update and restart',
      run: () =>
        void act(
          `update:${worker}`,
          () => trigger<ReconcileResult>('compose::update', withFile({ worker })),
          (r) => reconcileSummary(`Updated ${worker}`, r),
        ),
    })

  const addWorker = () => {
    const worker = workerInput.trim()
    if (!worker) return
    void act(
      `add:${worker}`,
      () => trigger<ReconcileResult>('compose::add', withFile({ worker })),
      (r) => {
        setWorkerInput('')
        return reconcileSummary(`Added ${worker}`, r)
      },
    )
  }

  const validate = () =>
    void act(
      'validate',
      () => trigger<Validation>('compose::validate', withFile({}), 30_000),
      (r) => {
        setValidation(r)
        return `Compose file is valid: ${r.start_order?.length ?? 0} containers in start order`
      },
    )

  const stopDaemon = () =>
    setConfirm({
      title: 'Stop the compose daemon?',
      description:
        'Every project it supervises goes down and the daemon exits. This page loses its data source until a daemon returns.',
      confirmLabel: 'Stop daemon',
      run: () =>
        void act(
          'daemon-stop',
          () => trigger<{ stopping?: string[] }>('compose::stop', {}, 60_000),
          () => 'Daemon stopping',
        ),
    })

  const toggleLogs = (container: string) => {
    if (expanded === container) {
      setExpanded(null)
      return
    }
    setExpanded(container)
    void loadLogs(container)
  }

  const focusFilter = useCallback(() => {
    setSection('containers')
    window.requestAnimationFrame(() => filterRef.current?.focus())
  }, [])

  const focusWorker = useCallback(() => {
    setSection('workers')
    window.requestAnimationFrame(() => workerRef.current?.focus())
  }, [])

  useEffect(
    () =>
      commands?.register([
        { id: 'refresh', title: 'Refresh status', run: () => void refresh() },
        { id: 'filter', title: 'Filter containers', run: focusFilter },
        { id: 'add', title: 'Add worker…', run: focusWorker },
        { id: 'validate', title: 'Validate compose file', enabled: () => !busy, run: validate },
        ...sections.map((s, index) => ({
          id: `section-${s.id}`,
          title: s.label,
          detail: s.description,
          run: () => setSection(s.id),
        })),
      ]),
    [commands, refresh, focusFilter, focusWorker, busy, validate],
  )

  const all = status?.containers ?? []
  const declared = useMemo(() => new Map((project?.containers ?? []).map((c) => [c.name, c])), [project])
  const topology = useMemo<TopologyInput>(
    () => ({
      namespace: status?.namespace ?? project?.namespace ?? null,
      file: status?.file ?? project?.file ?? null,
      engine: {
        url: project?.engine_url ?? null,
        host: project?.engine_host ?? null,
        port: project?.engine_port ?? null,
        pid: status?.daemon_pid ?? null,
      },
      containers: all.map((c) => {
        const d = declared.get(c.container)
        return {
          name: c.container,
          state: c.state,
          pid: running(c.state) ? (c.pid ?? null) : null,
          source: d?.source ?? null,
          ref: d?.ref ?? null,
          version: d?.version ?? null,
          ports: d?.ports.map((port) => port.port) ?? [],
          startAfter: d?.start_after ?? [],
          lastError: c.last_error ?? null,
        }
      }),
    }),
    [all, declared, project, status],
  )
  const selectedContainer = selectedNode ? (all.find((c) => c.container === selectedNode) ?? null) : null
  const selectedDeclared = selectedNode ? declared.get(selectedNode) : undefined
  useEffect(() => {
    if (loaded && selectedNode && !all.some((c) => c.container === selectedNode)) setSelectedNode(null)
  }, [loaded, selectedNode, all])
  const counts = useMemo(() => {
    const next = { ready: 0, starting: 0, failed: 0, stopped: 0 }
    for (const c of all) if (c.state in next) next[c.state as ContainerState] += 1
    return next
  }, [all])
  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return needle ? all.filter((c) => c.container.toLowerCase().includes(needle)) : all
  }, [all, query])

  const health = loaded ? describeHealth(counts, all.length) : null

  const sectionMeta = (id: Section): ReactNode => {
    if (!loaded) return null
    if (id === 'topology' && counts.failed) return <Chip tone="danger">{counts.failed} failing</Chip>
    if (id === 'overview' && health) return <Chip tone={health.tone}>{health.label}</Chip>
    if (id === 'containers') return <Chip tone={counts.failed ? 'danger' : 'neutral'}>{all.length}</Chip>
    if (id === 'daemon' && projects?.projects?.length) return <Chip tone="neutral">{projects.projects.length}</Chip>
    return null
  }

  const overview = (
    <>
      <SectionCard title="Project" actions={health ? <Chip tone={health.tone}>{health.label}</Chip> : null}>
        {!loaded ? (
          <LoadingRows rows={4} />
        ) : (
          <>
            <div className="cu-stats">
              <Stat
                value={
                  <>
                    {counts.ready}
                    <em>/{all.length}</em>
                  </>
                }
                label="ready"
              />
              <Stat value={counts.starting} label="starting" tone={counts.starting ? 'warn' : undefined} />
              <Stat value={counts.failed} label="failed" tone={counts.failed ? 'alert' : undefined} />
              <Stat value={counts.stopped} label="stopped" />
            </div>
            <Facts
              items={[
                { term: 'Namespace', children: <span className="cu-mono">{status?.namespace ?? '–'}</span> },
                { term: 'Daemon pid', children: <span className="cu-mono">{status?.daemon_pid ?? '–'}</span> },
                {
                  term: 'Engine',
                  children: <span className="cu-mono">{project?.engine_url ?? 'compose default'}</span>,
                },
                {
                  term: 'Timeouts',
                  children: (
                    <span className="cu-mono">
                      start {project?.startup_timeout ?? '–'} · stop {project?.stop_timeout ?? '–'}
                    </span>
                  ),
                },
                {
                  term: 'Ports',
                  children: portSummary(project),
                  wide: true,
                },
                { term: 'Compose file', children: <span className="cu-mono">{status?.file ?? '–'}</span>, wide: true },
                {
                  term: 'State directory',
                  children: <span className="cu-mono">{status?.state_dir ?? '–'}</span>,
                  wide: true,
                },
              ]}
            />
            <div className="cu-actions">
              <Button variant="primary" disabled={!!busy} onClick={() => void lifecycle('up')}>
                <Play />
                Start project
              </Button>
              <Button variant="ghost" disabled={!!busy} onClick={() => confirmLifecycle('restart')}>
                <RotateCw />
                Restart project
              </Button>
              <Button variant="ghost" disabled={!!busy} onClick={() => confirmLifecycle('down')}>
                <Square />
                Stop project
              </Button>
              <Button variant="pill" disabled={!!busy} onClick={validate}>
                <ShieldCheck />
                Validate file
              </Button>
            </div>
          </>
        )}
      </SectionCard>
      {validation ? (
        <SectionCard title="Validation" actions={<Chip tone="success">valid</Chip>}>
          <Facts
            items={[
              { term: 'Namespace', children: validation.namespace ?? '–' },
              {
                term: 'Start order',
                children: (
                  <ol className="cu-order">
                    {(validation.start_order ?? []).map((name) => (
                      <li key={name} className="cu-mono">
                        {name}
                      </li>
                    ))}
                  </ol>
                ),
                wide: true,
              },
              ...(validation.deferred_packages?.length
                ? [
                    {
                      term: 'Deferred packages',
                      children: <span className="cu-mono">{validation.deferred_packages.join(', ')}</span>,
                      wide: true,
                    },
                  ]
                : []),
            ]}
          />
        </SectionCard>
      ) : null}
    </>
  )

  const containers = (
    <SectionCard
      title={
        <>
          Containers
          {loaded ? (
            <span className="cu-count">
              {visible.length === all.length ? all.length : `${visible.length} of ${all.length}`}
            </span>
          ) : null}
        </>
      }
      actions={
        <Input
          ref={filterRef}
          aria-label="Filter containers"
          placeholder="Filter containers…"
          value={query}
          onChange={setQuery}
          className="cu-filter"
          data-autofocus=""
        />
      }
    >
      {!loaded ? (
        <LoadingRows rows={6} />
      ) : visible.length === 0 ? (
        <EmptyState
          icon={Boxes}
          title={query ? 'No containers match' : 'No containers'}
          description={
            query
              ? `Nothing matches “${query}”.`
              : 'This compose file declares no containers yet. Add a worker to start one.'
          }
          action={
            query
              ? { label: 'Clear filter', onClick: () => setQuery('') }
              : { label: 'Add worker', onClick: focusWorker }
          }
        />
      ) : (
        <TableViewport>
          <TableFrame>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Container</TableHead>
                  <TableHead>State</TableHead>
                  <TableHead>PID</TableHead>
                  <TableHead>Supervision</TableHead>
                  <TableHead>Source</TableHead>
                  <TableHead>Ports</TableHead>
                  <TableHead className="cu-actions-head">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {visible.map((c) => {
                  const open = expanded === c.container
                  const tail = logs[c.container]
                  const rowBusy = busy?.endsWith(`:${c.container}`) ?? false
                  return [
                    <TableRow key={c.container} selected={open}>
                      <TableCell>
                        <span className="cu-name">
                          <StatusDot tone={styleFor(c.state).dot} pulse={c.state === 'starting'} />
                          <span className="cu-mono cu-strong">{c.container}</span>
                        </span>
                        {c.last_error ? (
                          <span className="cu-last-error" title={c.last_error}>
                            {c.last_error}
                          </span>
                        ) : null}
                      </TableCell>
                      <TableCell>
                        <Badge variant={styleFor(c.state).badge}>{rowBusy ? `${c.state}…` : c.state}</Badge>
                      </TableCell>
                      <TableCell className="cu-mono cu-num">{running(c.state) ? (c.pid ?? '–') : '–'}</TableCell>
                      <TableCell className="cu-faint">{supervision(c)}</TableCell>
                      <TableCell>{sourceCell(declared.get(c.container))}</TableCell>
                      <TableCell className="cu-mono cu-num cu-ports">{portsCell(declared.get(c.container))}</TableCell>
                      <TableCell>
                        <div className="cu-row-actions">
                          {running(c.state) ? (
                            <IconButton
                              label={`Stop ${c.container}`}
                              variant="ghost"
                              disabled={!!busy}
                              onClick={() => confirmLifecycle('down', c.container)}
                            >
                              <Square />
                            </IconButton>
                          ) : (
                            <IconButton
                              label={`Start ${c.container}`}
                              variant="ghost"
                              disabled={!!busy}
                              onClick={() => void lifecycle('up', c.container)}
                            >
                              <Play />
                            </IconButton>
                          )}
                          <IconButton
                            label={`Restart ${c.container}`}
                            variant="ghost"
                            disabled={!!busy}
                            onClick={() => confirmLifecycle('restart', c.container)}
                          >
                            <RotateCw />
                          </IconButton>
                          <IconButton
                            label={open ? `Hide ${c.container} log` : `Show ${c.container} log`}
                            variant="ghost"
                            aria-expanded={open}
                            onClick={() => toggleLogs(c.container)}
                          >
                            <FileText />
                          </IconButton>
                        </div>
                      </TableCell>
                    </TableRow>,
                    open ? (
                      <TableRow key={`${c.container}:log`} className="cu-log-row">
                        <TableCell colSpan={7}>
                          <div className="cu-log">
                            <div className="cu-log-head">
                              <span className="cu-mono cu-faint">
                                {tail && 'path' in tail ? tail.path : `${c.container}.log`}
                                {tail && 'truncated' in tail && tail.truncated
                                  ? ` · last ${tail.lines.length} lines`
                                  : ''}
                              </span>
                              <IconButton label="Reload log" variant="ghost" onClick={() => void loadLogs(c.container)}>
                                <RefreshCw />
                              </IconButton>
                            </div>
                            {!tail ? (
                              <LoadingRows rows={3} />
                            ) : 'error' in tail ? (
                              <StatusPanel variant="alert" headline="Log unavailable" detail={tail.error} />
                            ) : tail.missing || tail.lines.length === 0 ? (
                              <p className="cu-note">
                                No log lines yet. The daemon creates the file once the container writes something.
                              </p>
                            ) : (
                              <TerminalStream
                                label={`${c.container}.log`}
                                text={tail.lines.join('\n')}
                                ansi
                                clampLines={40}
                                clampChars={20_000}
                              />
                            )}
                          </div>
                        </TableCell>
                      </TableRow>
                    ) : null,
                  ]
                })}
              </TableBody>
            </Table>
          </TableFrame>
        </TableViewport>
      )}
    </SectionCard>
  )

  const workers = (
    <SectionCard title="Worker packages">
      <p className="cu-note">
        Declare a worker by registry name, <span className="cu-mono">name@version</span>, or a local path. Compose
        resolves its dependencies, pins versions in the compose file, and reconciles only what changed.
      </p>
      <div className="cu-fields">
        <Field id="cu-worker-reference" label="Worker reference" hint="e.g. web, web@1.2.10, ../my-worker">
          <Input
            id="cu-worker-reference"
            ref={workerRef}
            placeholder="web, web@1.2.10, or ../my-worker"
            value={workerInput}
            onChange={setWorkerInput}
            onKeyDown={(event) => {
              if (event.key === 'Enter') addWorker()
            }}
            data-autofocus=""
          />
        </Field>
      </div>
      <div className="cu-actions">
        <Button variant="primary" disabled={!workerInput.trim() || !!busy} onClick={addWorker}>
          <Plus />
          Add worker
        </Button>
        <Button
          variant="ghost"
          disabled={!workerInput.trim() || !!busy}
          onClick={() => updateWorker(workerInput.trim())}
        >
          Update to latest
        </Button>
        <Button
          variant="ghost"
          disabled={!workerInput.trim() || !!busy}
          onClick={() => removeWorker(workerInput.trim())}
        >
          Remove
        </Button>
      </div>
      {loaded && all.length ? (
        <>
          <p className="cu-note">Declared workers. Pick one to update or remove it.</p>
          <div className="cu-chips">
            {all.map((c) => (
              <Chip
                key={c.container}
                tone={c.state === 'failed' ? 'danger' : 'neutral'}
                selected={workerInput.trim() === c.container}
                onClick={() => setWorkerInput(c.container)}
                role="button"
                tabIndex={0}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault()
                    setWorkerInput(c.container)
                  }
                }}
              >
                {c.container}
              </Chip>
            ))}
          </div>
        </>
      ) : null}
    </SectionCard>
  )

  const topologySection = (
    <>
      <SectionCard
        title={
          <>
            Topology
            {loaded ? <span className="cu-count">{all.length}</span> : null}
          </>
        }
        actions={health ? <Chip tone={health.tone}>{health.label}</Chip> : null}
      >
        {!loaded ? (
          <LoadingRows rows={5} />
        ) : all.length === 0 ? (
          <EmptyState
            icon={Boxes}
            title="Nothing to draw"
            description="This compose file declares no containers yet. Add a worker to start one."
            action={{ label: 'Add worker', onClick: focusWorker }}
          />
        ) : (
          <>
            <p className="cu-note">
              Ordered by <span className="cu-mono">start_after</span>. Pick a container to trace what it needs and what
              depends on it.
            </p>
            <Topology input={topology} selected={selectedNode} onSelect={setSelectedNode} />
          </>
        )}
      </SectionCard>
      {selectedContainer ? (
        <SectionCard
          title={
            <>
              <StatusDot tone={styleFor(selectedContainer.state).dot} pulse={selectedContainer.state === 'starting'} />
              <span className="cu-mono">{selectedContainer.container}</span>
              <Badge variant={styleFor(selectedContainer.state).badge}>{selectedContainer.state}</Badge>
            </>
          }
          actions={
            <div className="cu-actions cu-detail-actions">
              {running(selectedContainer.state) ? (
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={!!busy}
                  onClick={() => confirmLifecycle('down', selectedContainer.container)}
                >
                  <Square />
                  Stop
                </Button>
              ) : (
                <Button
                  variant="primary"
                  size="sm"
                  disabled={!!busy}
                  onClick={() => void lifecycle('up', selectedContainer.container)}
                >
                  <Play />
                  Start
                </Button>
              )}
              <Button
                variant="ghost"
                size="sm"
                disabled={!!busy}
                onClick={() => confirmLifecycle('restart', selectedContainer.container)}
              >
                <RotateCw />
                Restart
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  setSection('containers')
                  setQuery('')
                  setExpanded(selectedContainer.container)
                  void loadLogs(selectedContainer.container)
                }}
              >
                <FileText />
                Log
              </Button>
              <IconButton label="Close details" variant="ghost" onClick={() => setSelectedNode(null)}>
                <X />
              </IconButton>
            </div>
          }
        >
          <div className="cu-detail-grid">
            <div className="cu-detail-fact">
              <span className="cu-detail-term">PID</span>
              <span className="cu-mono cu-detail-value">
                {running(selectedContainer.state) ? (selectedContainer.pid ?? '–') : '–'}
              </span>
            </div>
            <div className="cu-detail-fact">
              <span className="cu-detail-term">Supervision</span>
              <span className="cu-detail-value">{supervision(selectedContainer)}</span>
            </div>
            <div className="cu-detail-fact">
              <span className="cu-detail-term">Listening</span>
              <span className="cu-mono cu-detail-value">
                {selectedDeclared?.ports.length
                  ? selectedDeclared.ports
                      .map((port) => `${port.address === '*' ? '' : `${port.address}:`}${port.port}`)
                      .join(', ')
                  : '–'}
              </span>
            </div>
            <div className="cu-detail-fact">
              <span className="cu-detail-term">Source</span>
              <span className="cu-mono cu-detail-value" title={selectedDeclared?.ref}>
                {selectedDeclared
                  ? selectedDeclared.source === 'package'
                    ? `${selectedDeclared.ref.split('/').pop()}${selectedDeclared.version ? `@${selectedDeclared.version}` : ''}`
                    : `${selectedDeclared.source}://${selectedDeclared.ref}`
                  : '–'}
              </span>
            </div>
          </div>
          <div className="cu-detail-relations">
            <div className="cu-detail-fact">
              <span className="cu-detail-term">Starts after</span>
              {selectedDeclared?.start_after.length ? (
                <span className="cu-chips">
                  {selectedDeclared.start_after.map((dep) => (
                    <NodeChip key={dep} name={dep} onPick={setSelectedNode} />
                  ))}
                </span>
              ) : (
                <span className="cu-detail-value cu-faint">the engine only</span>
              )}
            </div>
            <div className="cu-detail-fact">
              <span className="cu-detail-term">Depended on by</span>
              {(() => {
                const dependents = (project?.containers ?? []).filter((d) =>
                  d.start_after.includes(selectedContainer.container),
                )
                return dependents.length ? (
                  <span className="cu-chips">
                    {dependents.map((d) => (
                      <NodeChip key={d.name} name={d.name} onPick={setSelectedNode} />
                    ))}
                  </span>
                ) : (
                  <span className="cu-detail-value cu-faint">nothing</span>
                )
              })()}
            </div>
          </div>
          {selectedDeclared?.run || selectedDeclared?.environment.length ? (
            <dl className="cu-detail-lines">
              {selectedDeclared.run ? (
                <div>
                  <dt>run</dt>
                  <dd className="cu-mono">{selectedDeclared.run}</dd>
                </div>
              ) : null}
              {selectedDeclared.environment.length ? (
                <div>
                  <dt>environment</dt>
                  <dd className="cu-mono">{selectedDeclared.environment.join('  ')}</dd>
                </div>
              ) : null}
            </dl>
          ) : null}
          {selectedContainer.last_error ? (
            <StatusPanel variant="alert" headline="Last error" detail={selectedContainer.last_error} />
          ) : null}
        </SectionCard>
      ) : null}
    </>
  )

  const daemon = (
    <SectionCard title="Daemon">
      {!loaded ? (
        <LoadingRows rows={4} />
      ) : (
        <>
          <Facts
            items={[
              { term: 'Daemon', children: <span className="cu-mono">{projects?.daemon ?? 'compose'}</span> },
              { term: 'Namespace', children: projects?.daemon_namespace ?? status?.namespace ?? '–' },
              {
                term: 'Process',
                children: <span className="cu-mono">{projects?.daemon_pid ?? status?.daemon_pid ?? '–'}</span>,
              },
              { term: 'Projects', children: String(projects?.projects?.length ?? 0) },
              {
                term: 'Engine',
                children: (
                  <span className="cu-mono">
                    {project?.engine_url ?? 'compose default'}
                    {project?.daemon_ports.length
                      ? ` · daemon listens ${project.daemon_ports.map((p) => p.port).join(', ')}`
                      : ''}
                  </span>
                ),
                wide: true,
              },
            ]}
          />
          {projects?.projects?.length ? (
            <TableViewport>
              <TableFrame>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Project</TableHead>
                      <TableHead>Compose file</TableHead>
                      <TableHead>Containers</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {projects.projects.map((p) => (
                      <TableRow key={`${p.namespace}:${p.file}`}>
                        <TableCell className="cu-strong">{p.namespace ?? '–'}</TableCell>
                        <TableCell className="cu-mono">{p.file ?? '–'}</TableCell>
                        <TableCell className="cu-num">
                          {(p.containers ?? []).filter((c) => c.state === 'ready').length}/{p.containers?.length ?? 0}{' '}
                          ready
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableFrame>
            </TableViewport>
          ) : null}
          <div className="cu-actions">
            <Button variant="ghost" disabled={!!busy} onClick={stopDaemon}>
              <Square />
              Stop daemon
            </Button>
          </div>
        </>
      )}
    </SectionCard>
  )

  const content = { topology: topologySection, overview, containers, workers, daemon }[section]

  return (
    <PageShell className="cu-shell">
      <PageHeader
        icon={<Layers />}
        title="Compose"
        description={
          status?.namespace ? <span className="cu-mono">{status.namespace}</span> : 'Worker project supervision'
        }
        actions={
          <>
            <span
              className="cu-live"
              role="status"
              title={updatedAt ? `Updated ${new Date(updatedAt).toLocaleTimeString()}` : undefined}
            >
              <StatusDot tone={unavailable ? 'warn' : 'accent'} pulse={loaded && !unavailable} aria-hidden />
              {unavailable ? 'daemon unreachable' : refreshing ? 'refreshing' : 'live'}
            </span>
            <Button variant="ghost" size="sm" disabled={refreshing} onClick={() => void refresh()}>
              <RefreshCw className={refreshing ? 'cu-spin' : undefined} />
              refresh
            </Button>
          </>
        }
        onClose={onRequestClose}
      />
      <PageBody side={panelSide}>
        <PageSidebar
          label="sections"
          side={panelSide}
          collapsible
          resizable
          storageKey="compose:sections"
          defaultWidth={236}
          minWidth={180}
          maxWidth={340}
          narrowBelow={640}
          narrowMode="drawer"
        >
          <List className="cu-side-list" aria-label="Compose sections">
            {sections.map((item) => {
              const Icon = item.icon
              return (
                <ListItem
                  key={item.id}
                  selected={section === item.id}
                  aria-current={section === item.id ? 'page' : undefined}
                  leading={<Icon className={uiClasses.icon} />}
                  label={item.label}
                  description={item.description}
                  trailing={sectionMeta(item.id)}
                  onClick={() => setSection(item.id)}
                />
              )
            })}
          </List>
        </PageSidebar>
        <PageMain className="cu-main">
          {unavailable ? (
            <StatusPanel
              variant="warn"
              headline="Compose daemon not reachable"
              detail={`${unavailable}. Start one with iii compose --up -f worker-compose.yaml in the project directory.`}
            />
          ) : null}
          {feedback ? (
            <StatusPanel
              variant={feedback.kind === 'error' ? 'alert' : 'success'}
              headline={feedback.kind === 'error' ? 'Compose request failed' : 'Done'}
              detail={feedback.text}
            />
          ) : null}
          <div className="cu-section">{content}</div>
        </PageMain>
      </PageBody>
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
        title={confirm?.title ?? ''}
        description={confirm?.description}
        confirmLabel={confirm?.confirmLabel}
        onConfirm={() => confirm?.run()}
      />
    </PageShell>
  )
}
