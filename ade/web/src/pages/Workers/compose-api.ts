import type { ExtensionIii } from '@/types/injectable-ui'
import { fetchRawWorkersSnapshot } from './api/workers'
import { mergeWorkers } from './lib/merge-workers'
import type { WorkerRow } from './types'

export type ContainerState =
  | 'starting'
  | 'ready'
  | 'restarting'
  | 'failed'
  | 'stopped'

export type Container = {
  container: string
  state: ContainerState | string
  owned?: boolean
  pid?: number | null
  last_error?: string | null
  log_path?: string
}

export type Status = {
  namespace?: string
  file?: string
  state_dir?: string
  daemon_pid?: number
  containers?: Container[]
}

export type DeclaredContainer = {
  name: string
  source: 'path' | 'package' | 'unknown'
  ref: string
  version: string | null
  start_after: string[]
  environment: string[]
  run: string | null
}
export type Project = {
  file: string
  namespace: string | null
  engine_url: string | null
  startup_timeout: string | null
  stop_timeout: string | null
  containers: DeclaredContainer[]
}
export type ProjectList = {
  daemon?: string
  daemon_namespace?: string
  daemon_pid?: number
  projects?: { namespace?: string; file?: string; containers?: Container[] }[]
}

/** The compose project, plus every worker connected to the engine (compose or not). */
export type Snapshot = {
  status: Status
  list: ProjectList | null
  project: Project | null
  workers: WorkerRow[]
  /** False when no compose daemon answered: only engine workers are listed. */
  composed: boolean
}

export type MutationOutcome = {
  status?: 'ok' | 'failed'
  changed?: boolean
  error?: { code: string; message: string } | null
}
export type Accepted = {
  operation_id: string
  requested: number
  status: string
}
export type OperationSnapshot = {
  operation_id: string
  status: 'running' | 'succeeded' | 'failed' | 'cancelled'
  phase: string
  completed: number
  total: number
  last_event?: {
    detail: string
    container?: string | null
    phase: string
  } | null
}

export type LogEntry = { message: string; stream: 'stdout' | 'stderr' }
export type LogCursor = { generation: string; offset: number }
export type LogsOutcome = {
  containers: {
    container: string
    cursor?: LogCursor | null
    entries: LogEntry[]
    truncated: boolean
  }[]
}

export type PackageVersion = {
  version: string
  tags: string[]
  created_at: string | null
}
export type Versions = {
  container: string
  reference: string
  declared: string | null
  versions: PackageVersion[]
}
export type RegistryWorker = {
  name: string
  version: string
  description: string
  dependencies: string[]
}

export type Manifest = {
  path: string
  name: string
  language: string | null
  description: string | null
  dependencies: string[]
  /** `scripts.start`: how compose starts it when the entry has no `run`. */
  start: string | null
  /** `bin`: the binary a Rust worker builds. */
  bin: string | null
}
export type Inspection = {
  path: string
  exists: boolean
  manifest: Manifest | null
  run_found: boolean | null
  checkouts: { path: string; branch: string | null }[]
  workers: Manifest[]
}

export type EnvVar = { key: string; value: string | null; secret: boolean }
export type ContainerEntry = {
  name: string
  worker: string
  version: string | null
  start_after: string[]
  env_file: string[]
  environment: EnvVar[]
  run: string | null
  config_override: string | null
}
export type EditPatch = {
  worker?: string
  run?: string
  start_after?: string[]
  environment?: { set: Record<string, string>; unset: string[] }
  config_override?: string
}

/** Every call the page makes, bound to the console's bus client and the project's compose file. */
export function composeApi(iii: ExtensionIii, file: () => string | undefined) {
  const call = <T>(
    fn: string,
    payload: Record<string, unknown> = {},
    timeoutMs = 15_000,
  ) => iii.trigger<T>(fn, payload, { timeoutMs })
  const withFile = (payload: Record<string, unknown> = {}) => {
    const current = file()
    return current ? { file: current, ...payload } : payload
  }
  return {
    /**
     * The compose project and the engine's connected workers. Either may be
     * missing — an engine without a compose daemon still lists its workers.
     */
    async snapshot(): Promise<Snapshot> {
      const [status, list, project, engine] = await Promise.all([
        call<Status>('compose::status', withFile(), 10_000).catch(() => null),
        call<ProjectList>('compose::list', {}, 10_000).catch(() => null),
        call<Project>('console::compose::project', withFile()).catch(
          () => null,
        ),
        fetchRawWorkersSnapshot().catch(() => null),
      ])
      if (!status && !engine)
        throw new Error('Neither the compose daemon nor the engine answered')
      return {
        status: status ?? {},
        composed: status !== null,
        list,
        project,
        workers: engine ? mergeWorkers(engine) : [],
      }
    },
    lifecycle: (verb: 'up' | 'down' | 'restart', container?: string) =>
      call<MutationOutcome>(
        `compose::${verb}`,
        withFile(container ? { container } : {}),
        600_000,
      ),
    validate: () =>
      call<{
        namespace: string
        start_order: string[]
        deferred_packages: string[]
      }>('compose::validate', withFile()),
    stopDaemon: () => call<{ stopping: string[] }>('compose::stop', {}, 30_000),
    add: (workers: (string | Record<string, unknown>)[]) =>
      call<Accepted>('compose::add', withFile({ workers }), 60_000),
    update: (workers: string[]) =>
      call<Accepted>('compose::update', withFile({ workers }), 60_000),
    remove: (workers: string[]) =>
      call<Accepted>('compose::remove', withFile({ workers }), 60_000),
    operation: (operation_id: string) =>
      call<OperationSnapshot>('compose::operation', { operation_id }, 10_000),
    cancel: (operation_id: string) =>
      call<{ cancelled: boolean }>('compose::cancel', { operation_id }, 10_000),
    logs: (
      container: string,
      cursor: LogCursor | null,
      tail: number,
      waitMs: number,
    ) =>
      call<LogsOutcome>(
        'compose::logs',
        withFile(
          cursor
            ? { container, cursors: { [container]: cursor }, wait_ms: waitMs }
            : { container, tail },
        ),
        waitMs + 10_000,
      ),
    versions: (container: string) =>
      call<Versions>('console::compose::versions', withFile({ container })),
    packageVersions: (name: string) =>
      call<Versions>('console::compose::versions', { name }),
    search: (query: string) =>
      call<{ workers: RegistryWorker[] }>('console::compose::search', {
        query,
      }),
    inspect: (path: string, run?: string | null) =>
      call<Inspection>(
        'console::compose::inspect',
        run ? { path, run } : { path },
      ),
    container: (container: string) =>
      call<ContainerEntry>(
        'console::compose::container',
        withFile({ container }),
      ),
    edit: (container: string, patch: EditPatch) =>
      call<Accepted>(
        'console::compose::edit',
        withFile({ container, ...patch }),
        60_000,
      ),
  }
}

export type ComposeApi = ReturnType<typeof composeApi>
