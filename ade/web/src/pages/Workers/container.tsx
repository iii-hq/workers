import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import {
  ArrowDownToLine,
  Copy,
  FileText,
  MoreHorizontal,
  Play,
  RotateCw,
  Square,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { AnsiText } from '@/components/ui/AnsiText'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/DropdownMenu'
import { EmptyState } from '@/components/ui/EmptyState'
import { Eyebrow } from '@/components/ui/Eyebrow'
import { IconButton } from '@/components/ui/IconButton'
import { SearchField } from '@/components/ui/SearchField'
import { StatusDot } from '@/components/ui/StatusDot'
import { StatusPanel } from '@/components/ui/StatusPanel'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/Tabs'
import { StatusBar, Toolbar } from '@/components/ui/Toolbar'
import { stopSupervisorWorker } from './api/workers'
import { WorkerSurface } from './components/WorkerSurface'
import type {
  ComposeApi,
  Container,
  ContainerEntry,
  DeclaredContainer,
  LogCursor,
  LogEntry,
  Project,
  Status,
} from './compose-api'
import type { Actions } from './index'
import {
  dependentsOf,
  draftFrom,
  isRunning,
  type SettingsDraft,
  settingsPatch,
  shortPath,
  toneFor,
} from './model'
import { SettingsTab } from './settings'
import { SourceTab } from './source'
import { MANAGEMENT_LABEL, type WorkerRow } from './types'

/** A log line with a stable key for the list. */
type Line = LogEntry & { id: number }

let lineSeq = 0
const withIds = (entries: LogEntry[]): Line[] =>
  entries.map((entry) => {
    lineSeq += 1
    return { ...entry, id: lineSeq }
  })

const TAIL = 200
const KEEP = 2000
const WAIT_MS = 5000

type Tab = 'log' | 'source' | 'settings' | 'functions'

export function ContainerView({
  api,
  actions,
  container,
  declared,
  project,
  status,
  latest,
  onSelect,
  onRemoved,
  setDirty,
  connected,
}: {
  api: ComposeApi
  actions: Actions
  container: Container
  declared: DeclaredContainer | null
  project: Project | null
  status: Status
  latest: string | null
  onSelect: (name: string) => void
  onRemoved: () => void
  setDirty?: (dirty: boolean | string) => void
  /** The container is connected to the engine, so it has functions to show. */
  connected: boolean
}) {
  const name = container.container
  const [tab, setTab] = useState<Tab>('log')
  // Settings keep their draft while another tab is open.
  const [entry, setEntry] = useState<ContainerEntry | null>(null)
  const [draft, setDraft] = useState<SettingsDraft | null>(null)
  const changes = entry && draft ? settingsPatch(entry, draft).changes : 0
  useEffect(() => {
    setDirty?.(changes ? `${name} settings` : false)
    return () => setDirty?.(false)
  }, [changes, name, setDirty])

  const tone = toneFor(container.state)
  const running = isRunning(container.state)
  const all = project?.containers ?? []
  const after = declared?.start_after ?? []
  const neededBy = dependentsOf(all, name)
  const state = new Map(
    (status.containers ?? []).map((c) => [c.container, c.state]),
  )
  const outdated =
    declared?.source === 'package' &&
    latest &&
    declared.version &&
    /^\d/.test(declared.version) &&
    latest !== declared.version

  const lifecycle = async (verb: 'up' | 'down' | 'restart') => {
    if (verb === 'down') {
      const ok = await actions.confirm({
        title: `Stop ${name}?`,
        description: neededBy.length
          ? `Compose stops ${name} and what depends on it: ${neededBy.join(', ')}.`
          : `Compose stops ${name}. Nothing depends on it.`,
        confirmLabel: 'Stop',
        tone: 'danger',
      })
      if (!ok) return
    }
    const label = {
      up: `Starting ${name}`,
      down: `Stopping ${name}`,
      restart: `Restarting ${name}`,
    }[verb]
    await actions.run(label, () => api.lifecycle(verb, name))
  }

  const remove = async () => {
    const ok = await actions.confirm({
      title: `Remove ${name}?`,
      description: `Compose takes ${name} and every start_after reference to it out of the compose file, then stops only ${name}.`,
      details: neededBy.length
        ? [`No longer waits for it: ${neededBy.join(', ')}`]
        : undefined,
      confirmLabel: `Remove ${name}`,
      tone: 'danger',
    })
    if (
      ok &&
      (await actions.track(`Removing ${name}`, () => api.remove([name])))
    )
      onRemoved()
  }

  const dependencyPills = (names: string[]) =>
    names.map((dep) => (
      <Button key={dep} variant="pill" size="sm" onClick={() => onSelect(dep)}>
        <StatusDot
          tone={toneFor(state.get(dep) ?? 'stopped').dot}
          aria-label={state.get(dep) ?? 'not running'}
        />
        <span className="wk-mono">{dep}</span>
      </Button>
    ))

  return (
    <div className="wk-view">
      <div className="wk-masthead">
        <div className="wk-identity">
          <div className="wk-title-row">
            <h2 className="wk-title wk-mono">{name}</h2>
            <Badge variant={tone.badge}>{container.state}</Badge>
            {outdated ? (
              <Badge variant="accent">{latest} available</Badge>
            ) : null}
          </div>
          <div className="wk-meta wk-mono">
            {declared?.source === 'path' ? (
              <span title={declared.ref}>path {shortPath(declared.ref)}</span>
            ) : null}
            {declared?.source === 'package' ? (
              <span>
                package {declared.ref} · {declared.version ?? 'unpinned'}
              </span>
            ) : null}
            {running && container.pid ? <span>pid {container.pid}</span> : null}
          </div>
        </div>
        <div className="wk-actions">
          {running ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={actions.busy}
              onClick={() => void lifecycle('down')}
            >
              <Square />
              Stop
            </Button>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              disabled={actions.busy}
              onClick={() => void lifecycle('up')}
            >
              <Play />
              Start
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            disabled={actions.busy}
            onClick={() => void lifecycle('restart')}
          >
            <RotateCw />
            Restart
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <IconButton label={`More actions for ${name}`} variant="ghost">
                <MoreHorizontal />
              </IconButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem
                disabled={actions.busy}
                onSelect={() => void remove()}
              >
                Remove from project…
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {container.last_error && !running ? (
        <StatusPanel
          variant="alert"
          headline={
            container.state === 'restarting'
              ? 'Exited; Compose will try again'
              : 'Last error'
          }
          detail={container.last_error}
          action={
            <Button
              variant="pill"
              size="sm"
              disabled={actions.busy}
              onClick={() => void lifecycle('restart')}
            >
              Restart
            </Button>
          }
        />
      ) : null}

      <dl className="wk-relations">
        <Eyebrow as="dt">Starts after</Eyebrow>
        <dd className="wk-pills">
          {after.length ? (
            dependencyPills(after)
          ) : (
            <span className="wk-faint">the engine only</span>
          )}
        </dd>
        <Eyebrow as="dt">Needed by</Eyebrow>
        <dd className="wk-pills">
          {neededBy.length ? (
            dependencyPills(neededBy)
          ) : (
            <span className="wk-faint">nothing</span>
          )}
        </dd>
      </dl>

      <Tabs
        value={tab}
        onValueChange={(next) => setTab(next as Tab)}
        className="wk-tabs"
      >
        <TabsList variant="line" aria-label={name}>
          <TabsTrigger value="log" icon={false}>
            Log
          </TabsTrigger>
          <TabsTrigger value="source" icon={false}>
            Source
          </TabsTrigger>
          <TabsTrigger value="settings" icon={false}>
            Settings{changes ? ` · ${changes}` : ''}
          </TabsTrigger>
          {connected ? (
            <TabsTrigger value="functions" icon={false}>
              Functions
            </TabsTrigger>
          ) : null}
        </TabsList>
        <TabsContent value="log" className="wk-tab">
          <LogTab api={api} name={name} logPath={container.log_path} />
        </TabsContent>
        <TabsContent value="source" className="wk-tab">
          {declared ? (
            <SourceTab
              api={api}
              actions={actions}
              declared={declared}
              onOpenSettings={() => setTab('settings')}
            />
          ) : (
            <EmptyState
              compact
              title="Not in the compose file"
              description={`${name} runs, but the compose file does not declare it.`}
            />
          )}
        </TabsContent>
        {connected ? (
          <TabsContent value="functions" className="wk-tab">
            <WorkerSurface name={name} />
          </TabsContent>
        ) : null}
        <TabsContent value="settings" className="wk-tab">
          <SettingsTab
            api={api}
            actions={actions}
            name={name}
            declared={declared}
            others={all.map((c) => c.name).filter((other) => other !== name)}
            entry={entry}
            draft={draft}
            onLoaded={(next) => {
              setEntry(next)
              setDraft(draftFrom(next))
            }}
            onDraft={setDraft}
            onRemove={() => void remove()}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function LogTab({
  api,
  name,
  logPath,
}: {
  api: ComposeApi
  name: string
  logPath?: string
}) {
  const [lines, setLines] = useState<Line[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [follow, setFollow] = useState(true)
  const [filter, setFilter] = useState('')
  const [attempt, setAttempt] = useState(0)
  const cursor = useRef<LogCursor | null>(null)
  const pane = useRef<HTMLPreElement>(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt is the Retry token
  useEffect(() => {
    let cancelled = false
    setError(null)
    api
      .logs(name, null, TAIL, 0)
      .then((result) => {
        if (cancelled) return
        const own = result.containers.find((c) => c.container === name)
        cursor.current = own?.cursor ?? null
        setLines(withIds(own?.entries ?? []))
      })
      .catch((cause) => !cancelled && setError(errorMessage(cause)))
    return () => {
      cancelled = true
    }
  }, [api, name, attempt])

  const loaded = lines !== null
  // biome-ignore lint/correctness/useExhaustiveDependencies: attempt restarts following after Retry
  useEffect(() => {
    if (!follow || !loaded) return
    let cancelled = false
    void (async () => {
      while (!cancelled && cursor.current) {
        try {
          const result = await api.logs(name, cursor.current, 0, WAIT_MS)
          if (cancelled) return
          const own = result.containers.find((c) => c.container === name)
          if (own?.cursor) cursor.current = own.cursor
          if (own?.entries.length)
            setLines((prev) =>
              [...(prev ?? []), ...withIds(own.entries)].slice(-KEEP),
            )
        } catch (cause) {
          if (!cancelled) setError(errorMessage(cause))
          return
        }
      }
    })()
    return () => {
      cancelled = true
    }
  }, [api, name, follow, loaded, attempt])

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase()
    return (lines ?? []).filter(
      (line) => !needle || line.message.toLowerCase().includes(needle),
    )
  }, [lines, filter])

  // biome-ignore lint/correctness/useExhaustiveDependencies: new visible lines are what scroll the pane
  useEffect(() => {
    const el = pane.current
    if (follow && el) el.scrollTop = el.scrollHeight
  }, [visible, follow])

  return (
    <div className="wk-log-tab">
      <Toolbar
        aria-label={`${name} log`}
        end={
          <>
            <Button
              variant="ghost"
              size="sm"
              aria-pressed={follow}
              onClick={() => setFollow((on) => !on)}
            >
              <ArrowDownToLine />
              Follow
            </Button>
            <IconButton
              label="Copy log"
              variant="ghost"
              disabled={!visible.length}
              onClick={() =>
                void copyText(visible.map((line) => line.message).join('\n'))
              }
            >
              <Copy />
            </IconButton>
          </>
        }
      >
        <SearchField
          className="wk-log-filter"
          aria-label="Filter log lines"
          placeholder="Filter lines"
          value={filter}
          onChange={setFilter}
        />
      </Toolbar>
      {error ? (
        <StatusPanel
          variant="alert"
          headline="The log is unavailable"
          detail={error}
          action={
            <Button
              variant="pill"
              size="sm"
              onClick={() => setAttempt((n) => n + 1)}
            >
              Retry
            </Button>
          }
        />
      ) : null}
      {lines && lines.length === 0 ? (
        <EmptyState
          compact
          icon={FileText}
          title="No output yet"
          description={`${name} has not written anything.`}
        />
      ) : (
        <pre
          ref={pane}
          className="wk-log"
          role="log"
          aria-label={`${name} log`}
          aria-busy={!lines}
        >
          {visible.map((line) => (
            <div key={line.id} data-stream={line.stream}>
              <AnsiText text={line.message.replace(/\n$/, '')} />
            </div>
          ))}
        </pre>
      )}
      <StatusBar end={follow ? 'following' : 'paused'}>
        {logPath ? (
          <span className="wk-mono" title={logPath}>
            {shortPath(logPath)}
          </span>
        ) : null}
        <span>
          {filter
            ? `${visible.length} of ${lines?.length ?? 0} lines`
            : `${lines?.length ?? 0} lines`}
        </span>
      </StatusBar>
    </div>
  )
}

/** A worker connected to the engine that compose does not run: what it is and what it registered. */
export function WorkerView({
  worker,
  actions,
  onStopped,
}: {
  worker: WorkerRow
  actions: Actions
  onStopped: () => void
}) {
  const tone = toneFor(worker.status === 'connected' ? 'ready' : worker.status)
  const stop = async () => {
    const ok = await actions.confirm({
      title: `Stop ${worker.name}?`,
      description:
        'The worker supervisor stops the process. Start it again from where it was launched.',
      confirmLabel: 'Stop',
      tone: 'danger',
    })
    if (!ok) return
    const stopped = await actions.run(`Stopping ${worker.name}`, async () => {
      await stopSupervisorWorker(worker.name)
      return { status: 'ok' }
    })
    if (stopped) onStopped()
  }
  return (
    <div className="wk-view">
      <div className="wk-masthead">
        <div className="wk-identity">
          <div className="wk-title-row">
            <h2 className="wk-title wk-mono">{worker.name}</h2>
            <Badge variant={tone.badge}>{worker.status}</Badge>
            <Badge>{MANAGEMENT_LABEL[worker.managementKind]}</Badge>
          </div>
          <div className="wk-meta wk-mono">
            {worker.runtime ? <span>{worker.runtime}</span> : null}
            {worker.version ? <span>{worker.version}</span> : null}
            {worker.pid ? <span>pid {worker.pid}</span> : null}
            {worker.ipAddress ? <span>{worker.ipAddress}</span> : null}
          </div>
        </div>
        <div className="wk-actions">
          <Button
            variant="ghost"
            size="sm"
            disabled={!worker.stopEnabled || actions.busy}
            title={worker.stopDisabledReason ?? undefined}
            onClick={() => void stop()}
          >
            <Square />
            Stop
          </Button>
        </div>
      </div>
      {worker.status === 'connected' ? (
        <WorkerSurface name={worker.name} />
      ) : (
        <EmptyState
          compact
          title="Not connected"
          description={`${worker.name} is not connected to the engine, so it registers nothing.`}
        />
      )}
    </div>
  )
}
