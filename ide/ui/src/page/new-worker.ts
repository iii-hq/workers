/* The New worker dialog's logic, free of React so the tests run it in
   node: the worker-name rule (the one coder::scaffold-worker enforces),
   the folder default, which file to open, the dialog's steps, and "Add to
   stack" over the compose daemon's own functions. */

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

export const POLL_MS = 1000
/** Ten minutes, compose::add's own default timeout. */
export const MAX_POLLS = 600
const LOG_TAIL = 40

/** coder::scaffold-worker returns only the worker's own container, so a
    missing http is declared here, as the -ade templates' worker-compose.yaml
    declares it. */
const HTTP_CONTAINER = { worker: 'package://http', version: 'latest', config_name: 'http' }

const pause = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

/** Adds a scaffolded worker (and the containers it requires that the stack
    lacks) and follows it until it runs or fails. compose::add keys a path
    container by its folder's last segment, which coder::scaffold-worker
    makes the worker name, and adding a name the stack already has would
    repoint that container at the new folder. So it is refused, unless
    `owned` says an earlier attempt of this flow added it: then a retry
    restarts it. */
export async function addToStack(
  trigger: Trigger,
  result: ScaffoldResult,
  onProgress: (phase: StackPhase) => void,
  { owned = false, sleep = pause }: { owned?: boolean; sleep?: (ms: number) => Promise<void> } = {},
): Promise<StackOutcome> {
  const key = result.name
  // A retry already owns the container, whatever fails before the first compose call.
  let sent = owned
  try {
    const before = await trigger<ComposeStatus>('compose::status', {})
    const declared = new Set(before.containers.map((container) => container.container))
    let operation: string | null = null
    if (declared.has(key)) {
      if (!owned) {
        return { ok: false, error: `a container named ${key} already exists in the stack`, logs: [], owned: false }
      }
      // `container` is never empty here: an empty key is never declared.
      await trigger('compose::restart', { container: key })
    } else {
      sent = true
      const missing = result.requires.filter((name) => !declared.has(name))
      const workers = [result.compose, ...missing.map((name) => (name === 'http' ? HTTP_CONTAINER : name))]
      operation = (await trigger<{ operation_id: string }>('compose::add', { workers })).operation_id
    }
    return followStart(trigger, key, operation, onProgress, { owned: sent, sleep })
  } catch (error) {
    return withLogs(trigger, key, errorMessage(error), sent)
  }
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

/** Follows container `key` until it runs or fails: through `operation`
    (compose::add's) while that runs, then by its compose::status state.
    Also what a coder::scaffold-worker call that started the worker
    follows, with the operation id it returned. */
export async function followStart(
  trigger: Trigger,
  key: string,
  operation: string | null,
  onProgress: (phase: StackPhase) => void,
  { owned = true, sleep = pause }: { owned?: boolean; sleep?: (ms: number) => Promise<void> } = {},
): Promise<StackOutcome> {
  const fail = (error: string) => withLogs(trigger, key, error, owned)
  try {
    for (let poll = 0; poll < MAX_POLLS; poll++) {
      // The operation first: once it has ended, the status read after it is final.
      let op: ComposeOperation | null = null
      if (operation !== null) {
        try {
          op = await trigger<ComposeOperation>('compose::operation', { progress_operation_id: operation })
        } catch {
          // An operation compose no longer knows (an old call): the
          // container's own state answers from here on.
          operation = null
        }
      }
      const status = await trigger<ComposeStatus>('compose::status', {})
      const container = status.containers.find((entry) => entry.container === key)
      if (container?.state === 'ready') return { ok: true }
      if (container?.state === 'failed') return fail(container.last_error ?? `${key} failed to start.`)
      if (op !== null && op.status !== 'running') return fail(op.last_event?.detail ?? `compose::add ${op.status}.`)
      // No operation to wait on and no container: nothing will start it.
      if (op === null && !container) return fail(`${key} is not in the stack.`)
      onProgress(container?.state === 'starting' ? 'starting' : 'installing')
      await sleep(POLL_MS)
    }
    return fail(`${key} did not start within ${(MAX_POLLS * POLL_MS) / 60_000} minutes.`)
  } catch (error) {
    return fail(errorMessage(error))
  }
}
