/**
 * The setup wizard's calls on the engine. Everything the wizard changes goes
 * through `runStep`, one `PlanStep` at a time, reporting progress as it goes
 * so the activity log shows the real operation — the compose phase of a
 * worker being added, the model count arriving — not a spinner.
 *
 * A key the user pastes travels browser → engine → `secrets::set` and is kept
 * nowhere else; a key found on the machine is imported by the secrets worker
 * itself (`secrets::import`), so its value never reaches the browser at all.
 */

import { resolveConfigurationFamily } from '@/lib/configuration-family'
import { fetchConsoleConfigValue } from '@/lib/console-config'
import { getIiiClient } from '@/lib/iii-client'
import { normalizeErrorMessage } from '@/lib/providers'
import { isMissingFunction, storeKey } from '@/lib/secrets'
import { fetchEngineWorkersList } from '@/pages/Workers/api/workers'
import { workerSource } from './catalog'
import { type Done, type WakeTrigger, waitForEvents } from './event-wait'
import type { PlanStep, ProviderState, ToolScan } from './plan'
import { setPath } from './plan'

export type OnboardingStatus = 'new' | 'dismissed' | 'completed'

export interface OnboardingState {
  status: OnboardingStatus
  updated_at: number
  completed_at?: number
  summary?: unknown
  /**
   * Whether the wizard may open by itself on this ADE: `false` where the
   * configuration's `onboarding.auto_open` or the worker's
   * `III_CONSOLE_ONBOARDING_AUTO_OPEN` turns it off (a deployed ADE).
   * Absent from a backend older than the switch.
   */
  auto_open?: boolean
}

/** Progress a running step reports; `progress` is 0..1 when known. */
export interface StepProgress {
  note?: string
  progress?: number
}

export interface StepResult {
  /** Shown beside the finished entry (`14 models`, `already running`). */
  note?: string
}

const COMPOSE_ADD_TIMEOUT_MS = 600_000
/** A worker built from source can take minutes to compile on first start. */
const WORKER_START_TIMEOUT_MS = 600_000
const MODELS_TIMEOUT_MS = 90_000
/** A local judge may download its model on first start. */
const JUDGE_TIMEOUT_MS = 600_000
/** The longest a judge provider waits for its model to load in one call. */
const JUDGE_LOAD_WAIT_MS = 300_000
const CONFIGURATION_TIMEOUT_MS = 30_000

/**
 * Every wait below is event-driven (see `./event-wait`): these are the quiet
 * spells after which it reads the state once more, in case an event was
 * missed — never a repeating timer.
 */
const COMPOSE_SILENCE_MS = 30_000
const WORKERS_SILENCE_MS = 30_000
const MODELS_SILENCE_MS = 15_000
const JUDGE_SILENCE_MS = 30_000
const CONFIGURATION_SILENCE_MS = 10_000

/** Compose streams an add's progress on this trigger type. */
const COMPOSE_OPERATION_TRIGGER = 'compose-operation'
/** The engine fires this when a worker connects, registers, or leaves. */
const WORKERS_AVAILABLE_TRIGGER = 'engine::workers-available'
/** ...and this when the function registry changes. */
const FUNCTIONS_AVAILABLE_TRIGGER = 'engine::functions-available'
/** Worker-manager lifecycle, where an engine still publishes it. */
const WORKER_LIFECYCLE_TRIGGER = 'worker'
const CONFIGURATION_TRIGGER = 'configuration'
const ROUTER_MODELS_CHANGED = 'router::models::changed'
const ROUTER_PROVIDER_CHANGED = 'router::provider::changed'

/**
 * An error as the wizard shows it: its own messages keep their sentence
 * case, an engine `{ code, message }` shows the message. (The console's
 * `normalizeErrorMessage` lowercases, which reads badly as a full sentence.)
 */
export function readableError(error: unknown): string {
  if (error instanceof Error) return error.message.replace(/^Error:\s*/i, '')
  if (error && typeof error === 'object') {
    const { message, code } = error as { message?: unknown; code?: unknown }
    if (typeof message === 'string' && message.trim()) return message.trim()
    if (typeof code === 'string' && code) return code
  }
  return normalizeErrorMessage(error)
}

/** `null` when the ADE backend predates the wizard. */
export async function fetchOnboardingState(): Promise<OnboardingState | null> {
  const client = await getIiiClient()
  try {
    return await client.trigger<OnboardingState>(
      'console::onboarding::get',
      {},
      { timeoutMs: 5_000 },
    )
  } catch (error) {
    if (isMissingFunction(error)) return null
    throw error
  }
}

export async function saveOnboardingState(
  status: OnboardingStatus,
  summary?: unknown,
): Promise<void> {
  const client = await getIiiClient()
  await client.trigger(
    'console::onboarding::set',
    summary === undefined ? { status } : { status, summary },
    { timeoutMs: 5_000 },
  )
}

export async function scanMachine(): Promise<ToolScan[]> {
  const client = await getIiiClient()
  const result = await client.trigger<{ tools?: ToolScan[] }>(
    'console::onboarding::scan',
    {},
    { timeoutMs: 15_000 },
  )
  return Array.isArray(result?.tools) ? result.tools : []
}

export async function installedWorkerNames(): Promise<Set<string>> {
  const list = await fetchEngineWorkersList()
  return new Set(
    list.workers
      .map((worker) => worker.name)
      .filter((name): name is string => typeof name === 'string'),
  )
}

export { detectKeys } from '@/lib/secrets'

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value ? value : undefined
}

/** `router::provider::list` joined with each provider's chat model count. */
export async function readProviderStates(): Promise<ProviderState[]> {
  const client = await getIiiClient()
  const [providers, models] = await Promise.all([
    client.trigger<{ providers?: unknown }>('router::provider::list', {}),
    client.trigger<{ models?: unknown }>('router::models::list', {}),
  ])
  const counts = new Map<string, number>()
  for (const raw of Array.isArray(models?.models) ? models.models : []) {
    const provider = asString(asRecord(raw)?.provider)
    if (provider) counts.set(provider, (counts.get(provider) ?? 0) + 1)
  }
  const out: ProviderState[] = []
  for (const raw of Array.isArray(providers?.providers)
    ? providers.providers
    : []) {
    const row = asRecord(raw)
    const id = asString(row?.id)
    if (!row || !id) continue
    const available = row.available !== false
    out.push({
      id,
      title: asString(row.display_name) ?? id,
      configured: row.configured === true,
      available,
      // The router keeps a removed provider's catalog; its models are not
      // usable until the worker is back, so they do not count.
      modelCount: available ? (counts.get(id) ?? 0) : 0,
      credentialSource: asString(row.credential_source),
      credentialRef: asString(row.credential_ref),
      credentialError: asString(row.credential_error),
      ownsAuthentication: asString(row.credential_env_var) === undefined,
    })
  }
  return out
}

export async function readConsoleConfig(): Promise<Record<
  string,
  unknown
> | null> {
  try {
    return await fetchConsoleConfigValue()
  } catch {
    return null
  }
}

interface ConfigurationListing {
  configurations?: { id: string; metadata?: unknown }[]
}

/** The live entry id of a worker family (`default-llm-router` for `llm-router`). */
async function configurationId(family: string): Promise<string | null> {
  const client = await getIiiClient()
  const listing = await client.trigger<ConfigurationListing>(
    'configuration::list',
    {},
  )
  const resolution = resolveConfigurationFamily(
    family,
    listing.configurations ?? [],
  )
  if (resolution.kind === 'ambiguous') {
    throw new Error(
      `more than one ${family} configuration is registered (${resolution.ids.join(', ')}); set it from Settings → Workers`,
    )
  }
  return resolution.kind === 'resolved' ? resolution.id : null
}

export interface RunContext {
  consoleConfig: Record<string, unknown> | null
  report: (progress: StepProgress) => void
  signal: { cancelled: boolean }
}

/** Execute one plan step. Throws a readable error on failure. */
export async function runStep(
  step: PlanStep,
  context: RunContext,
): Promise<StepResult> {
  switch (step.kind) {
    case 'add-workers':
      return addWorkers(step.workers, context)
    case 'store-secret':
      return storeSecret(step)
    case 'set-config':
      return setConfigurationValue(
        step.configuration,
        step.path,
        step.value,
        context.signal,
      )
    case 'wait-models':
      return waitForModels(step.providerId, step.title, context)
    case 'check-judge':
      return checkJudge(step.title, step.hosted, context)
  }
}

interface OperationSnapshot {
  status: 'running' | 'succeeded' | 'failed' | 'cancelled'
  phase: string
  completed: number
  total: number
  last_event?: { detail?: string; container?: string | null } | null
}

/** One `compose-operation` event (iii-compose `ProgressEvent`). */
export interface ComposeProgressEvent {
  operation_id: string
  phase: string
  detail: string
  container?: string | null
  current?: number | null
  total?: number | null
  terminal: boolean
}

function asComposeEvent(payload: unknown): ComposeProgressEvent | null {
  const row = asRecord(payload)
  const operationId = asString(row?.operation_id)
  if (!row || !operationId) return null
  const number = (value: unknown) =>
    typeof value === 'number' && Number.isFinite(value) ? value : undefined
  return {
    operation_id: operationId,
    phase: typeof row.phase === 'string' ? row.phase : '',
    detail: typeof row.detail === 'string' ? row.detail : '',
    container: asString(row.container) ?? null,
    current: number(row.current),
    total: number(row.total),
    terminal: row.terminal === true,
  }
}

/** The activity-log line for one compose event: `phase · container · detail`. */
export function composeEventProgress(
  event: ComposeProgressEvent,
): StepProgress {
  const container =
    event.container && !event.detail.includes(event.container)
      ? event.container
      : undefined
  const { current, total } = event
  return {
    note: [event.phase, container, event.detail].filter(Boolean).join(' · '),
    // `total` is a tree depth on some phases; only a real count is progress.
    progress:
      current != null && total != null && total > 0 && current <= total
        ? current / total
        : undefined,
  }
}

/**
 * The terminal event names no status; it is read from `compose::operation`.
 * When that read fails, its detail is the last word (iii-compose
 * `ADD_DETAILS`: success says the workers are ready, a partial success says
 * it still succeeded).
 */
function terminalDetailOutcome(detail: string): Done<undefined> {
  if (/all requested workers are ready|still succeeded/i.test(detail)) {
    return { value: undefined }
  }
  throw new Error(detail || 'compose failed')
}

/** A caller-chosen operation id, so its events can be bound before it starts. */
function newOperationId(): string {
  const id =
    typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
  return `ade-setup:${id}`
}

function operationTrigger(operationId: string): WakeTrigger {
  return {
    type: COMPOSE_OPERATION_TRIGGER,
    config: { operation_id: operationId, terminal_only: false },
  }
}

/**
 * `compose::add` and follow its operation to the end through
 * `compose-operation` events: bound before the add starts, one
 * `compose::operation` read to catch up, one more when the terminal event
 * arrives (it carries no status), and one after a long silence.
 */
async function composeAdd(
  sources: readonly (string | Record<string, unknown>)[],
  { report, signal }: Pick<RunContext, 'report' | 'signal'>,
): Promise<void> {
  const client = await getIiiClient()
  const requested = newOperationId()
  // `null` once compose answered without an operation (finished in-line).
  let operationId: string | null = requested
  let terminal: ComposeProgressEvent | null = null
  let heard = false
  await waitForEvents<undefined>({
    handler: 'iii::console::onboarding::compose',
    triggers: [operationTrigger(requested)],
    start: async (arm) => {
      report({ note: 'adding to worker-compose.yaml' })
      const accepted = await client.trigger<{ operation_id?: string }>(
        'compose::add',
        { workers: sources, operation_id: requested },
        { timeoutMs: COMPOSE_ADD_TIMEOUT_MS },
      )
      const id = accepted?.operation_id ?? null
      if (id && id !== requested) {
        // An older compose ignored ours: follow the id it chose.
        operationId = id
        await arm(operationTrigger(id))
      } else if (!id) {
        operationId = null
      }
    },
    onEvent: (payload) => {
      const event = asComposeEvent(payload)
      if (!event || event.operation_id !== operationId) return 'ignore'
      heard = true
      report(composeEventProgress(event))
      if (!event.terminal) return 'progress'
      terminal = event
      return 'check'
    },
    check: async (cause) => {
      if (!operationId) return { value: undefined }
      let snapshot: OperationSnapshot
      try {
        snapshot = await client.trigger<OperationSnapshot>(
          'compose::operation',
          { operation_id: operationId },
          { timeoutMs: 10_000 },
        )
      } catch (error) {
        if (terminal) return terminalDetailOutcome(terminal.detail)
        throw error
      }
      const detail = snapshot.last_event?.detail
      if (!heard) {
        report({
          note: [snapshot.phase, detail].filter(Boolean).join(' · '),
          progress:
            snapshot.total > 0
              ? snapshot.completed / snapshot.total
              : undefined,
        })
      }
      if (snapshot.status === 'failed' || snapshot.status === 'cancelled') {
        throw new Error(detail || `compose ${snapshot.status}`)
      }
      if (snapshot.status === 'succeeded') return { value: undefined }
      // Still running by the snapshot although the event said it ended.
      if (cause.kind === 'event' && terminal) {
        return terminalDetailOutcome(terminal.detail)
      }
      return null
    },
    timeoutMs: COMPOSE_ADD_TIMEOUT_MS,
    onTimeout: () => {
      throw new Error('compose did not finish adding them in time')
    },
    silenceMs: COMPOSE_SILENCE_MS,
    signal,
  })
}

/**
 * Wait until every worker in `names` is connected: re-read the worker list
 * once now and once per `engine::workers-available` (a worker connected or
 * registered) — or worker-manager lifecycle — event.
 */
async function waitForWorkers(
  names: readonly string[],
  { report, signal }: Pick<RunContext, 'report' | 'signal'>,
): Promise<void> {
  let waiting = [...names]
  await waitForEvents<undefined>({
    handler: 'iii::console::onboarding::workers',
    triggers: [
      { type: WORKERS_AVAILABLE_TRIGGER },
      {
        type: WORKER_LIFECYCLE_TRIGGER,
        config: { operations: ['add'], stages: ['done'] },
      },
    ],
    check: async () => {
      const connected = await installedWorkerNames()
      waiting = names.filter((worker) => !connected.has(worker))
      if (waiting.length === 0) return { value: undefined }
      report({ note: `waiting for ${waiting.join(', ')} to connect` })
      return null
    },
    timeoutMs: WORKER_START_TIMEOUT_MS,
    onTimeout: () => {
      throw new Error(`${waiting.join(', ')} did not start in time`)
    },
    silenceMs: WORKERS_SILENCE_MS,
    signal,
  })
}

async function addWorkers(
  workers: readonly string[],
  { consoleConfig, report, signal }: RunContext,
): Promise<StepResult> {
  const before = await installedWorkerNames()
  const missing = workers.filter((worker) => !before.has(worker))
  if (missing.length === 0) return { note: 'already running' }
  const sources = missing.map((worker) => workerSource(worker, consoleConfig))
  await composeAdd(sources, { report, signal })
  await waitForWorkers(missing, { report, signal })
  return { note: `${missing.join(', ')} running` }
}

async function storeSecret(
  step: Extract<PlanStep, { kind: 'store-secret' }>,
): Promise<StepResult> {
  await storeKey(
    step.name,
    step.input,
    step.consumers,
    `Added by the ADE setup wizard for ${step.consumers.join(', ')}`,
  )
  // The masked hint tells which key it was; the reference it got does not
  // belong in setup's log.
  return step.input.mode === 'stored' || step.input.mode === 'env'
    ? {}
    : { note: 'saved' }
}

/**
 * The entry id of `family` once its worker registered it: re-listed once per
 * `configuration` registered/updated event. The binding names no
 * configuration id on purpose — a per-id binding keeps that entry's TTL
 * slot, and dropping it would start the entry's expiry countdown.
 */
function waitForConfiguration(
  family: string,
  signal?: RunContext['signal'],
): Promise<string> {
  const missing = () =>
    new Error(`the ${family} configuration is not registered`)
  return waitForEvents<string>({
    handler: 'iii::console::onboarding::configuration',
    triggers: [
      {
        type: CONFIGURATION_TRIGGER,
        config: {
          event_types: ['configuration:registered', 'configuration:updated'],
        },
      },
    ],
    check: async () => {
      const id = await configurationId(family)
      return id ? { value: id } : null
    },
    timeoutMs: CONFIGURATION_TIMEOUT_MS,
    onTimeout: async () => {
      const id = await configurationId(family)
      if (!id) throw missing()
      return id
    },
    silenceMs: CONFIGURATION_SILENCE_MS,
    signal,
  })
}

async function setConfigurationValue(
  family: string,
  path: readonly string[],
  value: string,
  signal?: RunContext['signal'],
): Promise<StepResult> {
  const client = await getIiiClient()
  // A worker added a moment ago registers its entry as it boots.
  const id =
    (await configurationId(family)) ??
    (await waitForConfiguration(family, signal))
  const current = await client.trigger<{ value?: unknown }>(
    'configuration::get',
    { id, raw: true },
  )
  await client.trigger('configuration::set', {
    id,
    value: setPath(current?.value ?? null, path, value),
  })
  return {}
}

/** The provider a `router::models::changed` / `router::provider::changed` event is about. */
function eventProvider(payload: unknown): string | undefined {
  return asString(asRecord(payload)?.provider)
}

/** The router's events about one provider; everything else is ignored. */
function providerEvents(providerId: string) {
  return {
    triggers: [
      { type: ROUTER_MODELS_CHANGED },
      { type: ROUTER_PROVIDER_CHANGED },
    ] satisfies WakeTrigger[],
    onEvent: (payload: unknown) =>
      eventProvider(payload) === providerId ? ('check' as const) : 'ignore',
  }
}

async function countProviderModels(providerId: string): Promise<number> {
  const client = await getIiiClient()
  const result = await client.trigger<{ models?: unknown[] }>(
    'router::models::list',
    { provider: providerId },
  )
  return Array.isArray(result?.models) ? result.models.length : 0
}

/**
 * Wait for the provider's chat models to reach the router: re-read once per
 * `router::models::changed` / `router::provider::changed` event about it (a
 * catalog reconciled, the provider registered or came back).
 */
async function waitForModels(
  providerId: string,
  title: string,
  { report, signal }: RunContext,
): Promise<StepResult> {
  const client = await getIiiClient()
  const found = (count: number): Done<StepResult> => ({
    value: { note: `${count} ${count === 1 ? 'model' : 'models'}` },
  })
  let state: ProviderState | undefined
  let asked = 0
  return waitForEvents<StepResult>({
    handler: 'iii::console::onboarding::models',
    ...providerEvents(providerId),
    check: async () => {
      const count = await countProviderModels(providerId)
      if (count > 0) return found(count)
      state = (await readProviderStates()).find(
        (provider) => provider.id === providerId,
      )
      if (state?.credentialError) throw new Error(state.credentialError)
      if (!state?.configured) {
        report({
          note: state
            ? 'waiting for the provider to list its models'
            : 'waiting for the provider to register',
        })
        return null
      }
      // Once the router holds a credential, ask the provider for its
      // catalog: a rejected key comes back as an empty list, and saying so
      // now beats a timeout a minute and a half later. The answer also
      // arrives as a `router::models::changed` event, which asks once more.
      report({ note: 'asking the provider for its models with this key' })
      const refreshed = await client
        .trigger<{ count?: number }>(
          `provider::${providerId}::refresh_models`,
          {},
          { timeoutMs: 30_000 },
        )
        .catch(() => null)
      asked++
      if ((refreshed?.count ?? 0) > 0) {
        const listed = await countProviderModels(providerId)
        if (listed > 0) return found(listed)
      } else if (asked >= 2) {
        throw new Error(
          `${title} returned no models for this key. Check that the key is valid and has API access, then connect again — or paste a different key.`,
        )
      }
      return null
    },
    timeoutMs: MODELS_TIMEOUT_MS,
    onTimeout: () => {
      throw new Error(
        state
          ? `${title} has not listed any models yet. Open the model picker to check its credentials.`
          : `${title} did not register with llm-router.`,
      )
    },
    silenceMs: MODELS_SILENCE_MS,
    signal,
  })
}

/** The hub's error for a provider that answered, as `judge::*` returns it. */
interface JudgeFailure {
  code?: string
  http_status?: number
  provider_error?: { detail?: { message?: string; error_type?: string } }
}

type JudgeAnswer = JudgeFailure & { models?: unknown[]; status?: string }

/**
 * The judge functions an `engine::functions-available` payload lists, as one
 * comparable string — the hub's and every `judge-<provider>`'s. A provider
 * registers its functions once it is ready (a local one after downloading
 * its model), which is the event worth re-asking on; every other registry
 * change (a console tab's handlers) is not.
 */
export function judgeFunctionsSignature(payload: unknown): string | null {
  const functions = asRecord(payload)?.functions
  if (!Array.isArray(functions)) return null
  return functions
    .map((row) => asString(asRecord(row)?.function_id))
    .filter((id): id is string => id?.startsWith('judge') === true)
    .sort()
    .join(',')
}

/**
 * Ask the judge hub for its models. A success proves the strategy is up and,
 * for a hosted judge, that its key works; a 401/403 is the key, said plainly.
 * The call itself waits for a local provider's model to load (up to the
 * provider's 5-minute cap); anything else (the provider not registered yet)
 * is asked again when the registry announces new judge functions.
 */
async function checkJudge(
  title: string,
  hosted: boolean,
  { report, signal }: RunContext,
): Promise<StepResult> {
  const client = await getIiiClient()
  const deadlineAt = Date.now() + JUDGE_TIMEOUT_MS
  let last = ''
  let seen: string | null = null
  // A provider whose operator lowered its timeout cap refuses the long
  // wait as `invalid_request`; ask with the hub's default from then on.
  let longWait = true
  const ask = async (): Promise<JudgeAnswer | null> => {
    const waitMs = Math.min(
      JUDGE_LOAD_WAIT_MS,
      Math.max(1_000, deadlineAt - Date.now()),
    )
    try {
      return await client.trigger<JudgeAnswer>(
        'judge::models::list',
        longWait ? { timeout_ms: waitMs } : {},
        { timeoutMs: (longWait ? waitMs : 30_000) + 10_000 },
      )
    } catch (error) {
      last = readableError(error)
      return error && typeof error === 'object' ? (error as JudgeAnswer) : null
    }
  }
  return waitForEvents<StepResult>({
    handler: 'iii::console::onboarding::judge',
    triggers: [{ type: FUNCTIONS_AVAILABLE_TRIGGER }],
    onEvent: (payload) => {
      const signature = judgeFunctionsSignature(payload)
      if (signature === null || signature === seen) return 'ignore'
      seen = signature
      return 'check'
    },
    check: async () => {
      report({ note: `asking ${title}` })
      last = ''
      // The hub answers a provider failure as a result with `status:
      // "error"`, and the bus rejects with the same shape; read both alike.
      let answer = await ask()
      if (longWait && answer?.code === 'invalid_request') {
        longWait = false
        last = ''
        answer = await ask()
      }
      // An auth failure is final: say so now instead of waiting it out.
      const failure = judgeFailure(answer, title, hosted)
      if (failure) throw new Error(failure)
      if (!last && answer?.status !== 'error') {
        return {
          value: {
            note: 'answering',
          },
        }
      }
      last = answer?.provider_error?.detail?.message ?? last
      report({
        note: last
          ? `waiting for ${title} — ${last}`
          : `waiting for ${title} to answer`,
      })
      return null
    },
    timeoutMs: JUDGE_TIMEOUT_MS,
    onTimeout: () => {
      throw new Error(
        `${title} did not answer in time${last ? `: ${last}` : ''}`,
      )
    },
    silenceMs: JUDGE_SILENCE_MS,
    signal,
  })
}

export function judgeFailure(
  value: unknown,
  title: string,
  hosted: boolean,
): string | null {
  if (!value || typeof value !== 'object') return null
  const { code, http_status, provider_error } = value as JudgeFailure
  const message = provider_error?.detail?.message?.trim().replace(/\.+$/, '')
  if (hosted && (http_status === 401 || http_status === 403)) {
    return `${title} rejected this key${message ? ` — ${message}` : ''}. Paste a different key and set up again.`
  }
  if (code === 'missing_key') {
    return message ?? `${title} has no key to use`
  }
  return null
}

/**
 * Add workers through compose and wait until they connect, reporting the
 * compose phase as it goes — the same action the wizard logs, for any
 * surface that needs a worker before it can continue (a key field without
 * the secrets worker, say).
 */
export async function addWorkersWithProgress(
  workers: readonly string[],
  report: (progress: StepProgress) => void,
): Promise<StepResult> {
  return addWorkers(workers, {
    consoleConfig: await readConsoleConfig(),
    report,
    signal: { cancelled: false },
  })
}

export interface ProviderKeyCheck {
  /** The router resolved a credential for the provider. */
  configured: boolean
  /** Chat models the provider listed with it. */
  models: number
  /** The router's reason when the credential did not resolve. */
  error?: string
}

/**
 * Right after a key changed: wait (briefly) for the router to resolve it,
 * then ask the provider for its catalog and count. A key the upstream
 * rejects comes back as an empty list — the provider swallows the 401 — so
 * zero models with a resolved key is the signal.
 */
export async function checkProviderKey(
  providerId: string,
  { settleMs = 6_000 }: { settleMs?: number } = {},
): Promise<ProviderKeyCheck> {
  const client = await getIiiClient()
  const read = async () =>
    (await readProviderStates()).find((provider) => provider.id === providerId)
  const settled = (state: ProviderState | undefined) =>
    !state || state.configured || state.credentialError !== undefined
  let state = await read()
  if (!settled(state)) {
    // The router resolves the new key, then has the provider re-list its
    // models: the `router::*::changed` events about it are the moments to
    // look again; at `settleMs` look one last time and take what is there.
    state = await waitForEvents<ProviderState | undefined>({
      handler: 'iii::console::provider-key',
      ...providerEvents(providerId),
      check: async () => {
        const next = await read()
        return settled(next) ? { value: next } : null
      },
      timeoutMs: settleMs,
      onTimeout: read,
    })
  }
  if (!state?.configured) {
    return { configured: false, models: 0, error: state?.credentialError }
  }
  const refreshed = await client
    .trigger<{ count?: number }>(
      `provider::${providerId}::refresh_models`,
      {},
      { timeoutMs: 30_000 },
    )
    .catch(() => null)
  const listed = await client
    .trigger<{ models?: unknown[] }>('router::models::list', {
      provider: providerId,
    })
    .catch(() => null)
  const models = Math.max(
    refreshed?.count ?? 0,
    Array.isArray(listed?.models) ? listed.models.length : 0,
  )
  return { configured: true, models }
}
