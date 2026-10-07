/* The New worker dialog's logic, free of React so the tests run it in
   node: the worker-name rule (the one coder::scaffold-worker enforces),
   the folder default, which file to open, the dialog's steps, and "Add to
   stack" over the compose daemon's own functions, followed through its
   `compose-operation` events rather than by asking again. */

import { errorMessage } from '@iii-dev/console-ui/format'
import { joinRel, stripDirSlash } from './paths'

export type Language = 'node' | 'python'

export const LANGUAGE_LABEL: Record<Language, string> = { node: 'Node', python: 'Python' }

/** One entry of coder::list-templates (ide/src/code/functions/list_templates.rs). */
export interface TemplateInfo {
  id: string
  name: string
  description: string
  language: Language
  /** Compose containers the worker needs, e.g. `http`. */
  requires: string[]
}

export interface TemplateSource {
  kind: 'dir' | 'git'
  location: string
  ref?: string | null
  revision?: string | null
  warning?: string | null
}

export interface ListTemplatesResult {
  source: TemplateSource
  templates: TemplateInfo[]
}

/** coder::scaffold-worker's output; `compose` is a ready compose::add object. */
export interface ScaffoldResult {
  template: string
  name: string
  /** Canonical absolute folder the files went to. */
  directory: string
  files: { path: string; bytes: number; revision: string }[]
  compose: Record<string, unknown>
  requires: string[]
  next_steps: string[]
  /** With start: the compose::add operation that installs and starts it. */
  operation_id?: string | null
  /** With start: the containers compose::add was asked to add, the worker first. */
  started?: string[]
  /** With start: why compose::add failed; the files are written. */
  start_error?: string | null
}

export const WORKER_NAME_RE = /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/
const WORKER_NAME_MAX = 63

/** null when `name` is a valid worker name, else what is wrong with it. */
export function validateWorkerName(name: string): string | null {
  if (name === '') return 'Enter a name.'
  if (name.length > WORKER_NAME_MAX) return `At most ${WORKER_NAME_MAX} characters.`
  if (!WORKER_NAME_RE.test(name)) return 'Lowercase letters, digits and single hyphens, starting with a letter.'
  return null
}

/** The folder a new worker goes to: `<parentDir>/<name>`, root-relative or
    absolute like `parentDir`. coder::scaffold-worker requires the last
    segment to be the name (C232): compose keys the container by it. */
export function defaultDirectory(parentDir: string, name: string): string {
  // stripDirSlash('/') is '', which would turn the absolute root root-relative.
  return parentDir === '/' ? `/${name}` : joinRel(stripDirSlash(parentDir), name)
}

/** The file to open once created: the shallowest src/index.ts or src/main.py. */
export function entryFile(files: string[]): string | null {
  const entries = files.filter((path) => /(^|\/)src\/(index\.ts|main\.py)$/.test(path))
  return entries.sort((a, b) => a.length - b.length)[0] ?? null
}

/** `iii-hq/templates@main · abc1234`, or `local: <dir>`. */
export function sourceLabel(source: TemplateSource): string {
  if (source.kind === 'dir') return `local: ${source.location}`
  const repo = source.location.replace(/^https:\/\/github\.com\//, '').replace(/\.git$/, '')
  const at = source.ref ? `${repo}@${source.ref}` : repo
  return source.revision ? `${at} · ${source.revision.slice(0, 7)}` : at
}

/* ── the dialog's steps ─────────────────────────────────────────────── */

export type StackPhase = 'installing' | 'starting'

export type NewWorkerStep = 'loading' | 'form' | 'creating' | 'result' | 'adding' | 'running' | 'failed'

export interface NewWorkerState {
  step: NewWorkerStep
  list: ListTemplatesResult | null
  /** The list, create or add error, shown in place. */
  error: string | null
  result: ScaffoldResult | null
  phase: StackPhase
  logs: string[]
  /** The failed add left its own container in the stack: Retry restarts it. */
  owned: boolean
}

export const NEW_WORKER_INITIAL: NewWorkerState = {
  step: 'loading',
  list: null,
  error: null,
  result: null,
  phase: 'installing',
  logs: [],
  owned: false,
}

/** The -ade templates register an ADE page with the worker's name as its id. */
export const hasAdePage = (templateId: string) => templateId.endsWith('-ade')

const withAdePage = (template: TemplateInfo) => hasAdePage(template.id)

/** The template to create from: `template` while the list still has it, else
    the first -ade one, else the first. */
export function pickTemplate(template: string | undefined, templates: TemplateInfo[]): string | undefined {
  if (template && templates.some((t) => t.id === template)) return template
  return (templates.find(withAdePage) ?? templates[0])?.id
}

/** The dialog's choices for one language: the -ade ones (the default pick)
    first, each titled without the language the switch above already names. */
export function templateChoices(templates: TemplateInfo[], language: Language) {
  const suffix = ` (${LANGUAGE_LABEL[language]})`
  return templates
    .filter((t) => t.language === language)
    .sort((a, b) => Number(withAdePage(b)) - Number(withAdePage(a)))
    .map((t) => ({ ...t, title: t.name.endsWith(suffix) ? t.name.slice(0, -suffix.length) : t.name }))
}

export type NewWorkerAction =
  | { type: 'load' }
  | { type: 'listed'; list: ListTemplatesResult }
  | { type: 'list-failed'; error: string }
  | { type: 'create' }
  | { type: 'created'; result: ScaffoldResult }
  | { type: 'create-failed'; error: string }
  | { type: 'add' }
  | { type: 'progress'; phase: StackPhase }
  | { type: 'added' }
  | { type: 'add-failed'; error: string; logs: string[]; owned: boolean }

/** An action that does not fit the current step (a late reply) changes nothing. */
export function newWorkerReducer(state: NewWorkerState, action: NewWorkerAction): NewWorkerState {
  const { step } = state
  switch (action.type) {
    case 'load':
      return step === 'loading' || step === 'form' ? { ...state, step: 'loading', error: null } : state
    case 'listed':
      return step === 'loading' ? { ...state, step: 'form', list: action.list } : state
    case 'list-failed':
      return step === 'loading' ? { ...state, step: 'form', error: action.error } : state
    case 'create':
      return step === 'form' ? { ...state, step: 'creating', error: null } : state
    case 'created':
      return step === 'creating' ? { ...state, step: 'result', result: action.result } : state
    case 'create-failed':
      return step === 'creating' ? { ...state, step: 'form', error: action.error } : state
    case 'add':
      return step === 'result' || step === 'failed'
        ? { ...state, step: 'adding', phase: 'installing', error: null, logs: [] }
        : state
    case 'progress':
      return step === 'adding' ? { ...state, phase: action.phase } : state
    case 'added':
      return step === 'adding' ? { ...state, step: 'running' } : state
    case 'add-failed':
      return step === 'adding'
        ? { ...state, step: 'failed', error: action.error, logs: action.logs, owned: action.owned }
        : state
  }
}

/* ── the result's progress ───────────────────────────────────────────── */

/** `live` is the last step's ongoing state: the worker runs. */
/** `skipped`: a step that will not run, after the one before it failed. */
export type StepState = 'done' | 'live' | 'active' | 'pending' | 'failed' | 'skipped'

export interface ProgressStep {
  label: string
  state: StepState
}

/** Install and Start, the two steps after the files: where "Add to stack"
    is, and which of them a failure stopped at. */
export function stackSteps({ step, phase }: Pick<NewWorkerState, 'step' | 'phase'>): [ProgressStep, ProgressStep] {
  const installed: ProgressStep = { label: 'Installed', state: 'done' }
  const start: ProgressStep = { label: 'Start', state: 'pending' }
  switch (step) {
    case 'running':
      return [installed, { label: 'Running', state: 'live' }]
    case 'adding':
      return phase === 'installing'
        ? [{ label: 'Installing…', state: 'active' }, start]
        : [installed, { label: 'Starting…', state: 'active' }]
    case 'failed':
      return phase === 'installing'
        ? [
            { label: 'Install failed', state: 'failed' },
            { label: 'Not started', state: 'skipped' },
          ]
        : [installed, { label: 'Did not start', state: 'failed' }]
    default:
      return [{ label: 'Install', state: 'pending' }, start]
  }
}

/** `8s`, `1m 05s`. */
export function formatElapsed(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000))
  if (seconds < 60) return `${seconds}s`
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`
}

/** One row of engine::functions::list, which leaves internal functions out. */
export interface FunctionEntry {
  function_id: string
  description?: string | null
}

/** The worker's own functions: its `<name>::` ids, in id order. */
export function workerFunctions(entries: FunctionEntry[], name: string): FunctionEntry[] {
  return entries
    .filter((entry) => entry.function_id.startsWith(`${name}::`))
    .sort((a, b) => a.function_id.localeCompare(b.function_id))
}

/* ── Add to stack ───────────────────────────────────────────────────── */

export type Trigger = <T>(functionId: string, payload: Record<string, unknown>) => Promise<T>

/** One `compose-operation` event (iii-compose's ProgressEvent). */
export interface ProgressEvent {
  operation_id: string
  /** The container the event is about; absent for the operation as a whole. */
  container?: string | null
  /** A container's `waiting`, `configuring`, `preparing`, `registering`,
      `starting`, `ready`, `failed` or `warning`; `complete` at the end. */
  phase: string
  detail: string
  /** The operation's last event, success or failure alike. */
  terminal: boolean
}

/** Bind `compose-operation` for one operation: `onEvent` hears each of its
    events until the returned unbind. */
export type Subscribe = (operationId: string, onEvent: (event: ProgressEvent) => void) => () => void

/** The client half of `Subscribe`: what the console's engine client offers. */
export interface EventClient {
  browserId: string
  on<P = unknown>(functionId: string, handler: (payload: P) => void | Promise<void>): () => void
  registerTrigger(input: { type: string; function_id: string; config: Record<string, unknown> }): () => void
}

let followers = 0

/** `Subscribe` over the console's engine client, one browser function per
    operation (the `iii::` prefix keeps its invocations out of traces). */
export function composeOperations(iii: EventClient): Subscribe {
  return (operationId, onEvent) => {
    followers += 1
    const functionId = `iii::shell-ui::compose-operation::${followers}`
    const offHandler = iii.on<ProgressEvent>(functionId, (event) => {
      if (event?.operation_id === operationId && typeof event.phase === 'string') onEvent(event)
    })
    let offTrigger: () => void = () => {}
    try {
      offTrigger = iii.registerTrigger({
        type: 'compose-operation',
        function_id: `${functionId}::${iii.browserId}`,
        config: { operation_id: operationId },
      })
    } catch {
      // No compose daemon to bind: the snapshot read after the start answers.
    }
    return () => {
      try {
        offTrigger()
      } finally {
        offHandler()
      }
    }
  }
}

/** A caller-chosen operation id: bound before compose::add runs it. */
export function newOperationId(): string {
  return `compose:${crypto.randomUUID()}`
}

export type StackOutcome = { ok: true } | { ok: false; error: string; logs: string[]; owned: boolean }

interface ComposeStatus {
  containers: { container: string; state: string; last_error?: string | null }[]
}

interface ComposeOperation {
  status: 'running' | 'succeeded' | 'failed' | 'cancelled'
  last_event?: { detail: string } | null
}

interface ComposeLogs {
  containers: { entries: { message: string }[] }[]
}

interface MutationOutcome {
  status?: 'ok' | 'failed'
  error?: { message?: string } | null
}

/** Ten minutes, compose::add's own default timeout: one timer, not a loop. */
export const GIVE_UP_MS = 600_000
const LOG_TAIL = 40
/** Container phases that mean its process is on its way up. */
const STARTING_PHASES = new Set(['starting', 'registering'])

/** coder::scaffold-worker returns only the worker's own container, so a
    missing http is declared here, as the -ade templates' worker-compose.yaml
    declares it. */
const HTTP_CONTAINER = { worker: 'package://http', version: 'latest', config_name: 'http' }

export interface FollowOptions {
  /** The container was added by this flow: a failure leaves it ours to restart. */
  owned?: boolean
  /** How long to wait for a verdict before giving up. */
  giveUpMs?: number
  /** Stops following (the dialog or the card closed). */
  signal?: AbortSignal
}

/** Adds a scaffolded worker (and the containers it requires that the stack
    lacks) and follows it until it runs or fails. compose::add keys a path
    container by its folder's last segment, which coder::scaffold-worker
    makes the worker name, and adding a name the stack already has would
    repoint that container at the new folder. So it is refused, unless
    `owned` says an earlier attempt of this flow added it: then a retry
    restarts it. */
export async function addToStack(
  trigger: Trigger,
  subscribe: Subscribe,
  result: ScaffoldResult,
  onProgress: (phase: StackPhase) => void,
  { owned = false, ...options }: FollowOptions & { operationId?: string } = {},
): Promise<StackOutcome> {
  const key = result.name
  // A retry already owns the container, whatever fails before the first compose call.
  let sent = owned
  try {
    const before = await trigger<ComposeStatus>('compose::status', {})
    const declared = new Set(before.containers.map((container) => container.container))
    if (declared.has(key)) {
      if (!owned) {
        return { ok: false, error: `a container named ${key} already exists in the stack`, logs: [], owned: false }
      }
      // `container` is never empty here: an empty key is never declared.
      // compose::restart answers once the container is back up, or not.
      onProgress('starting')
      const restarted = await trigger<MutationOutcome>('compose::restart', { container: key })
      const failed = restarted?.status === 'failed' ? (restarted.error?.message ?? `${key} did not restart.`) : null
      return await afterRestart(trigger, key, failed)
    }
    sent = true
    const missing = result.requires.filter((name) => !declared.has(name))
    const workers = [result.compose, ...missing.map((name) => (name === 'http' ? HTTP_CONTAINER : name))]
    const operation = options.operationId ?? newOperationId()
    return await followStart(trigger, subscribe, key, operation, onProgress, {
      ...options,
      owned: true,
      start: async () => {
        await trigger('compose::add', { workers, operation_id: operation })
      },
    })
  } catch (error) {
    return withLogs(trigger, key, errorMessage(error), sent)
  }
}

/** A restart that returned: the container's state is the verdict. */
async function afterRestart(trigger: Trigger, key: string, failed: string | null): Promise<StackOutcome> {
  const status = await trigger<ComposeStatus>('compose::status', {})
  const container = status.containers.find((entry) => entry.container === key)
  if (container?.state === 'failed')
    return withLogs(trigger, key, container.last_error ?? `${key} failed to start.`, true)
  if (failed !== null && container?.state !== 'ready') return withLogs(trigger, key, failed, true)
  return { ok: true }
}

/** A failed outcome, with the container's last log lines when it has any. */
async function withLogs(trigger: Trigger, key: string, error: string, owned: boolean): Promise<StackOutcome> {
  let logs: string[] = []
  try {
    const out = await trigger<ComposeLogs>('compose::logs', { container: key, tail: LOG_TAIL })
    logs = out.containers.flatMap((container) => container.entries.map((entry) => entry.message.trimEnd()))
  } catch {
    // No logs (the container never got declared): the error says enough.
  }
  return { ok: false, error, logs, owned }
}

/** Follows container `key` through compose operation `operation` until it
    runs or fails. The operation's events are bound first, then `start`
    (when given) submits it, then its snapshot is read once: an operation
    that ended before the binding took still answers. From there on only
    events move it: the container's `starting`, `ready` or `failed`, and the
    operation's terminal event, after which the container's state is read
    once for the verdict. Also what a coder::scaffold-worker call that
    started the worker follows, with the operation id it returned. */
export function followStart(
  trigger: Trigger,
  subscribe: Subscribe,
  key: string,
  operation: string,
  onProgress: (phase: StackPhase) => void,
  { owned = true, giveUpMs = GIVE_UP_MS, signal, start }: FollowOptions & { start?: () => Promise<void> } = {},
): Promise<StackOutcome> {
  return new Promise<StackOutcome>((resolve) => {
    let done = false
    let settling = false
    let off: () => void = () => {}
    let timer: ReturnType<typeof setTimeout> | null = null
    const finish = (outcome: StackOutcome | Promise<StackOutcome>) => {
      if (done) return
      done = true
      off()
      if (timer !== null) clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      resolve(outcome)
    }
    const fail = (error: string) => finish(withLogs(trigger, key, error, owned))
    const abort = () => finish({ ok: false, error: 'stopped following the start', logs: [], owned })
    // The operation is over, or compose no longer knows it: one read of
    // the container's state decides.
    const settle = async (op: ComposeOperation | null, known: boolean) => {
      if (done || settling) return
      settling = true
      try {
        const status = await trigger<ComposeStatus>('compose::status', {})
        if (done) return
        const container = status.containers.find((entry) => entry.container === key)
        if (container?.state === 'ready') return finish({ ok: true })
        if (container?.state === 'failed') return fail(container.last_error ?? `${key} failed to start.`)
        if (!known) {
          return fail(
            container
              ? `compose no longer tracks the operation that starts ${key}; see the Workers page.`
              : `${key} is not in the stack.`,
          )
        }
        fail(op?.last_event?.detail ?? `compose::add ${op?.status ?? 'ended'}.`)
      } catch (error) {
        fail(errorMessage(error))
      }
    }
    const snapshot = () =>
      trigger<ComposeOperation>('compose::operation', { progress_operation_id: operation }).then(
        (op) => ({ op, known: true }),
        () => ({ op: null, known: false }),
      )

    if (signal?.aborted) return abort()
    signal?.addEventListener('abort', abort)
    timer = setTimeout(() => fail(`${key} did not start within ${giveUpMs / 60_000} minutes.`), giveUpMs)
    off = subscribe(operation, (event) => {
      if (done) return
      if (event.container === key) {
        if (event.phase === 'ready') return finish({ ok: true })
        if (event.phase === 'failed') return fail(event.detail || `${key} failed to start.`)
        if (STARTING_PHASES.has(event.phase)) onProgress('starting')
      }
      if (event.terminal) void snapshot().then(({ op, known }) => settle(op, known))
    })
    void (async () => {
      try {
        if (start) await start()
        if (done) return
        const { op, known } = await snapshot()
        if (done) return
        if (!known || op === null || op.status !== 'running') return void settle(op, known)
        onProgress('installing')
      } catch (error) {
        fail(errorMessage(error))
      }
    })()
  })
}
