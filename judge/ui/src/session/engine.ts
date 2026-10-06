import type { ExtensionIii } from '@iii-dev/console-ui'
import { BUILT_IN_PROVIDER, listProviders } from '../configuration'

export type Engine = Pick<ExtensionIii, 'trigger'>

/** How often an open picker checks whether an added judge registered. */
export const ADD_POLL_MS = 3_000
/** An added judge that has not registered by then is reported as stuck. */
export const ADD_GIVE_UP_MS = 10 * 60_000
/**
 * How long a judge whose add succeeded gets to show up in the list. Compose
 * reports success once the worker is ready, but also when a worker that is
 * not required failed to start, or was already declared and is stopped.
 */
const REGISTER_GRACE_MS = 15_000
/**
 * How long an open picker keeps checking after an add fails: a judge that
 * registers late (after the grace, or once its daemon restarted) still
 * turns into Added.
 */
export const FAILED_WATCH_MS = 2 * 60_000

export function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** A judge provider running on the engine, with what its settings say. */
export interface JudgeProvider {
  /** `typesafe` for the `judge-typesafe` worker. */
  provider: string
  worker: string
  /** Its configuration entry (`judge-<provider>::configuration-id`), when it answers. */
  configurationId: string | null
  /** The model its settings name, when they name one. */
  model: string | null
}

/** The hub's configuration entry and the default provider it stores. */
export interface JudgeSettings {
  configurationId: string
  provider: string
}

async function configurationIdOf(iii: Engine, functionId: string): Promise<string | null> {
  const identity = await iii.trigger<{ id?: unknown }>(functionId, {}, { timeoutMs: 5_000 })
  return typeof identity?.id === 'string' && identity.id ? identity.id : null
}

/**
 * The `model` a configuration entry names. Read raw, so a `${VAR}` stays a
 * template here instead of expanding (with whatever else the entry holds)
 * into the browser.
 */
async function configuredModel(iii: Engine, configurationId: string): Promise<string | null> {
  const entry = await iii.trigger<{ value?: { model?: unknown } | null }>(
    'configuration::get',
    { id: configurationId, raw: true },
    { timeoutMs: 5_000 },
  )
  const model = entry?.value?.model
  return typeof model === 'string' && model.trim() ? model : null
}

/**
 * Every registered judge provider with its configuration entry and model.
 * Nothing here asks a provider for its models: a local one would start
 * loading gigabytes just because the picker opened.
 */
export async function listJudges(iii: Engine): Promise<JudgeProvider[]> {
  const registered = await listProviders(iii)
  return Promise.all(
    registered.map(async ({ provider, worker }) => {
      const configurationId = await configurationIdOf(iii, `judge-${provider}::configuration-id`).catch(() => null)
      const model = configurationId ? await configuredModel(iii, configurationId).catch(() => null) : null
      return { provider, worker, configurationId, model }
    }),
  )
}

export async function readJudgeSettings(iii: Engine): Promise<JudgeSettings | null> {
  const configurationId = await configurationIdOf(iii, 'judge::configuration-id')
  if (!configurationId) return null
  const entry = await iii.trigger<{ value?: { provider?: unknown } }>(
    'configuration::get',
    { id: configurationId },
    { timeoutMs: 5_000 },
  )
  const provider = entry?.value?.provider
  return { configurationId, provider: typeof provider === 'string' && provider ? provider : BUILT_IN_PROVIDER }
}

/** A compose operation's latest snapshot (`compose::operation`). */
interface OperationSnapshot {
  status?: string
  last_event?: { terminal?: boolean; detail?: string } | null
}

/**
 * Why a compose operation failed. A worker with no build for this platform
 * is the common case, and its detail wraps the reason in advice for the
 * publisher, so only that sentence is shown.
 */
function failureReason(detail: string | undefined): string {
  if (!detail) return 'compose::add failed'
  return /[^.]*does not support platform[^.]*\./.exec(detail)?.[0].trim() ?? detail
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/**
 * `compose::add` only accepts the work: the operation resolves, installs and
 * starts the worker afterwards. Follow it until it ends, so a failure (no
 * build for this platform, a registry error) shows instead of a registration
 * that never comes. Success is settled by the judge registering; `fail` only
 * touches an add still in progress.
 */
async function followOperation(iii: Engine, worker: string, operationId: string, fail: (error: string) => void) {
  const deadline = Date.now() + ADD_GIVE_UP_MS
  while (Date.now() < deadline) {
    await sleep(ADD_POLL_MS)
    const snapshot = await iii
      .trigger<OperationSnapshot>('compose::operation', { operation_id: operationId }, { timeoutMs: 5_000 })
      .catch(() => null)
    if (snapshot?.status === 'failed') return fail(failureReason(snapshot.last_event?.detail))
    if (snapshot?.status === 'cancelled') return fail('The add was cancelled.')
    if (snapshot?.last_event?.terminal) {
      await sleep(REGISTER_GRACE_MS)
      return fail(`${worker} was added but has not started; check its logs in Settings → Workers.`)
    }
  }
}

/** Start `compose::add` for `worker` and report a failure through `fail`. */
export function addWorker(iii: Engine, worker: string, fail: (error: string) => void) {
  iii
    .trigger<{ status?: string; operation_id?: string; error?: { message?: string } | null }>(
      'compose::add',
      { workers: [worker] },
      { timeoutMs: 600_000 },
    )
    .then((reply) => {
      if (reply?.status === 'failed') fail(reply.error?.message ?? 'compose::add failed')
      else if (reply?.operation_id) void followOperation(iii, worker, reply.operation_id, fail)
    })
    .catch((error: unknown) => fail(message(error)))
}
