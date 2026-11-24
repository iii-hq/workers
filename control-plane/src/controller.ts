import { randomUUID } from 'node:crypto'
import type { ControlPlaneClient, ControlPlaneConfig, ControlPlaneData } from './types.js'
import { reasoningOptions } from './writer.js'

export const CONTROLLER_PROFILE = 'control-plane-controller'
const STATE_SCOPE = 'control-plane'

/**
 * Unrestricted for now, at the person's request (2026-11-24): every function
 * the engine offers, in every scope. The repository's permission rules
 * (iii-permissions.yaml) still deny agents harness::send, stop, run and
 * compact. The board-only policy of controller.md (C1, C2) comes back with
 * phase 3; the profile still confirms consequential actions first.
 */
export const controllerPolicy = (_scope: string) => ['*']

interface StoredController {
  session_id: string
  created_at: string
}

export interface OpenedController {
  session_id: string
  created: boolean
  /** The effort the new chat starts on, when one is set and the console accepts it. */
  thinking_level?: string
}

export interface ControllerDeps {
  iii: Pick<ControlPlaneClient, 'call'>
  config: () => ControlPlaneConfig
  read: (scope: string) => Promise<ControlPlaneData>
}

/** The model the controller runs on: its own, else the summary's. */
export function controllerModel(config: ControlPlaneConfig) {
  const model = config.controller?.model ?? config.summary.model
  // An effort picked for the controller wins, also when its model follows the summary's.
  const reasoning =
    config.controller?.reasoning ??
    (config.controller?.model ? undefined : config.summary.reasoning)
  return { model, reasoning }
}

/** The efforts the console chat accepts; any other value stays a provider option of the first send. */
const CHAT_LEVELS = ['minimal', 'low', 'medium', 'high', 'xhigh']
export const chatThinkingLevel = (reasoning: string | undefined) =>
  reasoning && CHAT_LEVELS.includes(reasoning) ? reasoning : undefined

/**
 * ponytail: opens are deduplicated in memory only, which assumes one control-plane worker per
 * namespace (compose runs one). Two instances opening the same scope at once could each create a
 * session; a conditional state write would close that if the worker ever scales out.
 *
 * One controller chat per scope. The first open creates the session with a
 * fresh, unguessable id, its model in the metadata the console reads, and a
 * first send that freezes the controller profile and its function policy;
 * later messages from the console chat inherit both. A deleted session is
 * replaced on the next open.
 */
export class ControllerSessions {
  private readonly opening = new Map<string, Promise<OpenedController>>()

  constructor(private readonly deps: ControllerDeps) {}

  open(scope: string): Promise<OpenedController> {
    const running = this.opening.get(scope)
    if (running) return running
    const next = this.openOnce(scope).finally(() => this.opening.delete(scope))
    this.opening.set(scope, next)
    return next
  }

  private async openOnce(scope: string): Promise<OpenedController> {
    const { iii } = this.deps
    const key = `controller:${scope}`
    const stored = await iii.call<StoredController | null>('state::get', {
      scope: STATE_SCOPE,
      key,
    })
    if (stored?.session_id) {
      // Only a missing (or deleted) session is replaced: session::get answers null for it.
      // An engine error propagates, so one timeout never orphans the person's chat.
      const existing = await iii.call<{ meta?: unknown } | null>('session::get', {
        session_id: stored.session_id,
      })
      // A reopened chat keeps the effort the person may have changed in it.
      if (existing) return { session_id: stored.session_id, created: false }
    }
    const { model, reasoning } = controllerModel(this.deps.config())
    if (!model)
      throw new Error('Pick a controller model (or a summary model) in the control plane settings.')
    const data = await this.deps.read(scope)
    const name =
      scope === 'all'
        ? 'All projects'
        : ((data.projects ?? []).find((project) => project.id === scope)?.name ?? scope)
    const sessionId = `lead-${scope}-${randomUUID()}`
    await iii.call('session::ensure', {
      session_id: sessionId,
      title: `Controller · ${name}`,
      kind: 'user',
      metadata: {
        origin: 'control-plane-controller',
        project_id: scope === 'all' ? null : scope,
        surface: 'console',
        model,
        ...(chatThinkingLevel(reasoning) ? { thinking_level: chatThinkingLevel(reasoning) } : {}),
      },
    })
    const [provider, ...modelParts] = model.split('::')
    try {
      const sent = await iii.call<{
        turn_id?: string
        merged?: boolean
        queued?: boolean
        deduplicated?: boolean
      }>('harness::send', {
        session_id: sessionId,
        idempotency_key: `controller:${sessionId}:open`,
        message: `Give me a short status of ${name} (scope ${scope}): what moved, what is running, and what needs me.`,
        provider,
        model: modelParts.join('::'),
        options: {
          agent: CONTROLLER_PROFILE,
          ...reasoningOptions(provider, reasoning),
          functions: { allow: controllerPolicy(scope) },
        },
      })
      // Only a new turn freezes the profile and the policy; anything else leaves a plain chat.
      if (!sent?.turn_id || sent.merged || sent.queued || sent.deduplicated)
        throw new Error('The controller chat did not start its first turn.')
      await iii.call('state::set', {
        scope: STATE_SCOPE,
        key,
        value: {
          session_id: sessionId,
          created_at: new Date().toISOString(),
        } satisfies StoredController,
      })
    } catch (error) {
      // A half-made session would be a chat without the profile, or one no open finds again.
      await iii
        .call('harness::delete-session-tree', { session_id: sessionId })
        .catch(() => undefined)
      throw error
    }
    const thinkingLevel = chatThinkingLevel(reasoning)
    return {
      session_id: sessionId,
      created: true,
      ...(thinkingLevel ? { thinking_level: thinkingLevel } : {}),
    }
  }
}
