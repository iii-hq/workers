import type { ExtensionIii } from '@iii-dev/console-ui'
import { BUILT_IN_PROVIDER, listProviders } from '../configuration'

/** The console's bus client: calls, plus the bindings that push changes. */
export type Engine = Pick<ExtensionIii, 'trigger' | 'on' | 'registerTrigger' | 'browserId'>

/** An added judge that has not registered by then is reported as stuck. */
export const ADD_GIVE_UP_MS = 10 * 60_000
/**
 * How long a judge whose add succeeded gets to show up in the list. Compose
 * reports success once the worker is ready, but also when a worker that is
 * not required failed to start, or was already declared and is stopped.
 */
export const REGISTER_GRACE_MS = 15_000

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

/** A `compose-operation` delivery; bound terminal-only, so the last one. */
interface ProgressEvent {
  operation_id?: string
  terminal?: boolean
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

let followSeq = 0

/**
 * Start `compose::add` for `worker` and report a failure through `fail`.
 *
 * `compose::add` only accepts the work: the operation resolves, installs and
 * starts the worker afterwards. Its end is pushed, never polled for: a
 * terminal-only `compose-operation` binding goes in BEFORE the add, under an
 * operation id chosen here, and `compose::operation` is read once after the
 * accept (an operation that ended before the binding landed) and once when
 * the terminal event arrives, for the status the event does not carry. A
 * failure (no build for this platform, a registry error) shows instead of a
 * registration that never comes. Success is settled by the judge registering;
 * `fail` only touches an add still in progress.
 */
export function addWorker(iii: Engine, worker: string, fail: (error: string) => void) {
  let operationId = `compose:${crypto.randomUUID()}`
  const handlerId = `iii::judge-ui::compose-operation::${++followSeq}`
  let settled = false
  let reading = false
  let offTrigger: (() => void) | null = null
  const unbind = () => {
    try {
      offTrigger?.()
    } catch {
      // already gone
    }
    offTrigger = null
    offHandler()
  }
  const settle = (snapshot: OperationSnapshot | null, error?: string) => {
    if (settled) return
    settled = true
    clearTimeout(giveUp)
    unbind()
    if (error !== undefined) return fail(error)
    if (snapshot?.status === 'failed') return fail(failureReason(snapshot.last_event?.detail))
    if (snapshot?.status === 'cancelled') return fail('The add was cancelled.')
    // Done: the judge registering settles it, unless it never does.
    setTimeout(
      () => fail(`${worker} was added but has not started; check its logs in Settings → Workers.`),
      REGISTER_GRACE_MS,
    )
  }
  const read = () =>
    iii
      .trigger<OperationSnapshot>('compose::operation', { operation_id: operationId }, { timeoutMs: 5_000 })
      .catch(() => null)
  // The one fallback: an operation that never ends (a daemon gone) is stuck.
  const giveUp = setTimeout(
    () => settle(null, 'Not registered after 10 minutes; check Settings → Workers.'),
    ADD_GIVE_UP_MS,
  )
  const bind = () => {
    try {
      offTrigger?.()
    } catch {
      // already gone
    }
    offTrigger = iii.registerTrigger({
      type: 'compose-operation',
      function_id: `${handlerId}::${iii.browserId}`,
      config: { operation_id: operationId, terminal_only: true },
    })
  }
  const offHandler = iii.on<ProgressEvent>(handlerId, (event) => {
    if (settled || reading || event?.operation_id !== operationId || event.terminal !== true) return
    reading = true
    void read().then((snapshot) => settle(snapshot))
  })
  bind()

  iii
    .trigger<{ status?: string; operation_id?: string; error?: { message?: string } | null }>(
      'compose::add',
      { workers: [worker], operation_id: operationId },
      { timeoutMs: 600_000 },
    )
    .then((reply) => {
      if (settled) return
      if (reply?.status === 'failed') return settle(null, reply.error?.message ?? 'compose::add failed')
      // A daemon that ignores the caller's id ran it under its own.
      if (reply?.operation_id && reply.operation_id !== operationId) {
        operationId = reply.operation_id
        bind()
      }
      // The catch-up read: an operation that already ended sends nothing more.
      return read().then((snapshot) => {
        if (settled || reading || !snapshot?.status || snapshot.status === 'running') return
        reading = true
        settle(snapshot)
      })
    })
    .catch((error: unknown) => settle(null, message(error)))
}
