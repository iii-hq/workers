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
import {
  DEFAULT_ENV_FILE,
  isMissingFunction,
  keyStore,
  storeKey,
} from '@/lib/secrets'
import { fetchEngineWorkersList } from '@/pages/Workers/api/workers'
import { workerSource } from './catalog'
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
const OPERATION_POLL_MS = 700
/** A worker built from source can take minutes to compile on first start. */
const WORKER_START_TIMEOUT_MS = 600_000
const MODELS_TIMEOUT_MS = 90_000
/** A local judge may download its model on first start. */
const JUDGE_TIMEOUT_MS = 600_000
const CONFIGURATION_TIMEOUT_MS = 30_000

const sleep = (ms: number) =>
  new Promise<void>((resolve) => window.setTimeout(resolve, ms))

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
      return setConfigurationValue(step.configuration, step.path, step.value)
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

async function addWorkers(
  workers: readonly string[],
  { consoleConfig, report, signal }: RunContext,
): Promise<StepResult> {
  const client = await getIiiClient()
  const before = await installedWorkerNames()
  const missing = workers.filter((worker) => !before.has(worker))
  if (missing.length === 0) return { note: 'already running' }
  const sources = missing.map((worker) => workerSource(worker, consoleConfig))
  report({ note: 'asking compose to declare them' })
  const accepted = await client.trigger<{ operation_id?: string }>(
    'compose::add',
    { workers: sources },
    { timeoutMs: COMPOSE_ADD_TIMEOUT_MS },
  )
  const operationId = accepted?.operation_id
  if (operationId) {
    for (;;) {
      if (signal.cancelled) throw new Error('cancelled')
      const snapshot = await client.trigger<OperationSnapshot>(
        'compose::operation',
        { operation_id: operationId },
        { timeoutMs: 10_000 },
      )
      const detail = snapshot.last_event?.detail
      report({
        note: [snapshot.phase, detail].filter(Boolean).join(' · '),
        progress:
          snapshot.total > 0 ? snapshot.completed / snapshot.total : undefined,
      })
      if (snapshot.status === 'failed' || snapshot.status === 'cancelled') {
        throw new Error(detail || `compose ${snapshot.status}`)
      }
      if (snapshot.status === 'succeeded') break
      await sleep(OPERATION_POLL_MS)
    }
  }
  const deadline = Date.now() + WORKER_START_TIMEOUT_MS
  for (;;) {
    const names = await installedWorkerNames()
    const waiting = missing.filter((worker) => !names.has(worker))
    if (waiting.length === 0) break
    if (signal.cancelled) throw new Error('cancelled')
    if (Date.now() > deadline) {
      throw new Error(`${waiting.join(', ')} did not start in time`)
    }
    report({ note: `waiting for ${waiting.join(', ')} to connect` })
    await sleep(1_500)
  }
  return { note: `${missing.join(', ')} running` }
}

async function storeSecret(
  step: Extract<PlanStep, { kind: 'store-secret' }>,
): Promise<StepResult> {
  const meta = await storeKey(
    step.name,
    step.input,
    step.consumers,
    `Added by the ADE setup wizard for ${step.consumers.join(', ')}`,
  )
  return {
    note:
      step.input.mode === 'stored' || step.input.mode === 'env'
        ? `${meta.consumers.join(', ')} can read it`
        : keyStore(step.input) === 'env'
          ? `written to ${step.envFile ?? DEFAULT_ENV_FILE} (${meta.hint})`
          : `stored ${meta.hint}`,
  }
}

async function setConfigurationValue(
  family: string,
  path: readonly string[],
  value: string,
): Promise<StepResult> {
  const client = await getIiiClient()
  // A worker added a moment ago registers its entry as it boots.
  const deadline = Date.now() + CONFIGURATION_TIMEOUT_MS
  let id = await configurationId(family)
  while (!id) {
    if (Date.now() > deadline) {
      throw new Error(`the ${family} configuration is not registered`)
    }
    await sleep(1_000)
    id = await configurationId(family)
  }
  const current = await client.trigger<{ value?: unknown }>(
    'configuration::get',
    { id, raw: true },
  )
  await client.trigger('configuration::set', {
    id,
    value: setPath(current?.value ?? null, path, value),
  })
  return { note: id }
}

async function waitForModels(
  providerId: string,
  title: string,
  { report, signal }: RunContext,
): Promise<StepResult> {
  const client = await getIiiClient()
  const deadline = Date.now() + MODELS_TIMEOUT_MS
  let asked = 0
  for (;;) {
    if (signal.cancelled) throw new Error('cancelled')
    const result = await client.trigger<{ models?: unknown[] }>(
      'router::models::list',
      { provider: providerId },
    )
    const count = Array.isArray(result?.models) ? result.models.length : 0
    if (count > 0) {
      return { note: `${count} ${count === 1 ? 'model' : 'models'}` }
    }
    const state = (await readProviderStates()).find(
      (provider) => provider.id === providerId,
    )
    if (state?.credentialError) throw new Error(state.credentialError)
    // Once the router holds a credential, ask the provider for its catalog
    // and wait for the answer: a rejected key comes back as an empty list,
    // and saying so now beats a timeout a minute and a half later.
    if (state?.configured) {
      report({ note: 'asking the provider for its models with this key' })
      const refreshed = await client
        .trigger<{ count?: number }>(
          `provider::${providerId}::refresh_models`,
          {},
          { timeoutMs: 30_000 },
        )
        .catch(() => null)
      asked++
      if ((refreshed?.count ?? 0) === 0 && asked >= 2) {
        throw new Error(
          `${title} returned no models for this key. Check that the key is valid and has API access, then connect again — or paste a different key.`,
        )
      }
    } else {
      report({
        note: state
          ? 'waiting for the provider to list its models'
          : 'waiting for the provider to register',
      })
    }
    if (Date.now() > deadline) {
      throw new Error(
        state
          ? `${title} has not listed any models yet. Open the model picker to check its credentials.`
          : `${title} did not register with llm-router.`,
      )
    }
    await sleep(1_500)
  }
}

/** The hub's error for a provider that answered, as `judge::*` returns it. */
interface JudgeFailure {
  code?: string
  http_status?: number
  provider_error?: { detail?: { message?: string; error_type?: string } }
}

type JudgeAnswer = JudgeFailure & { models?: unknown[]; status?: string }

/**
 * Ask the judge hub for its models. A success proves the strategy is up and,
 * for a hosted judge, that its key works; a 401/403 is the key, said plainly.
 * Anything else (the provider still starting, a model still downloading) is
 * waited out.
 */
async function checkJudge(
  title: string,
  hosted: boolean,
  { report, signal }: RunContext,
): Promise<StepResult> {
  const client = await getIiiClient()
  const deadline = Date.now() + JUDGE_TIMEOUT_MS
  for (;;) {
    if (signal.cancelled) throw new Error('cancelled')
    // The hub answers a provider failure as a result with `status: "error"`,
    // and the bus rejects with the same shape; read both the same way.
    let answer: JudgeAnswer | null
    let last = ''
    try {
      answer = await client.trigger<JudgeAnswer>(
        'judge::models::list',
        {},
        { timeoutMs: 30_000 },
      )
    } catch (error) {
      answer =
        error && typeof error === 'object' ? (error as JudgeAnswer) : null
      last = readableError(error)
    }
    // An auth failure is final: say so now instead of waiting it out.
    const failure = judgeFailure(answer, title, hosted)
    if (failure) throw new Error(failure)
    if (!last && answer?.status !== 'error') {
      const count = Array.isArray(answer?.models) ? answer.models.length : 0
      return {
        note:
          count > 0
            ? `answering · ${count} ${count === 1 ? 'model' : 'models'}`
            : 'answering',
      }
    }
    last = answer?.provider_error?.detail?.message ?? last
    if (Date.now() > deadline) {
      throw new Error(
        `${title} did not answer in time${last ? `: ${last}` : ''}`,
      )
    }
    report({
      note: last
        ? `waiting for ${title} — ${last}`
        : `waiting for ${title} to answer`,
    })
    await sleep(2_000)
  }
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
  const deadline = Date.now() + settleMs
  let state = (await readProviderStates()).find(
    (provider) => provider.id === providerId,
  )
  while (state && !state.configured && !state.credentialError) {
    if (Date.now() > deadline) break
    await sleep(500)
    state = (await readProviderStates()).find(
      (provider) => provider.id === providerId,
    )
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
