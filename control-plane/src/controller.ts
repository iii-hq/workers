import { randomUUID } from 'node:crypto'
import type { ControlPlaneClient, ControlPlaneConfig, ControlPlaneData } from './types.js'
import { reasoningOptions } from './writer.js'

export const CONTROLLER_PROFILE = 'control-plane-controller'
const STATE_SCOPE = 'control-plane'

/** What every controller reads: the control plane, the board, sessions and PRs. */
export const CONTROLLER_READS = [
  'control-plane::digest',
  'control-plane::overview',
  'control-plane::needs-you::list',
  'control-plane::runs::list',
  'control-plane::prs::list',
  'control-plane::decided::list',
  'control-plane::assumptions::list',
  'kanban::board::get',
  'kanban::project::list',
  'kanban::release::list',
  'kanban::ticket::get',
  'kanban::ticket::list',
  'kanban::comment::list',
  'kanban::activity::list',
  'kanban::question::list',
  'session::list',
  'session::messages-tail',
  'harness::status',
  'harness::ask::list',
  'github::pr::list',
  'github::pr::view',
  'github::pr::checks',
  // To confirm a batch of board actions with the person before writing.
  'harness::ask',
] as const

/**
 * A project controller acts through the board only: creating, updating and
 * moving cards (To do starts a card's run through dispatch), commenting and
 * recording decisions. harness::send, harness::stop and answering a question
 * stay out (controller.md C1, C2).
 */
export const CONTROLLER_WRITES = [
  'kanban::ticket::create',
  'kanban::ticket::update',
  'kanban::ticket::move',
  'kanban::comment::create',
  'kanban::question::record',
] as const

/** "All projects" reads only: a write needs one project's controller. */
export const controllerPolicy = (scope: string) => [
  ...CONTROLLER_READS,
  ...(scope === 'all' ? [] : CONTROLLER_WRITES),
]

interface StoredController {
  session_id: string
  created_at: string
}

export interface ControllerDeps {
  iii: Pick<ControlPlaneClient, 'call'>
  config: () => ControlPlaneConfig
  read: (scope: string) => Promise<ControlPlaneData>
}

/** The model the controller runs on: its own, else the summary's. */
export function controllerModel(config: ControlPlaneConfig) {
  const model = config.controller?.model ?? config.summary.model
  const reasoning = config.controller?.model
    ? config.controller.reasoning
    : config.summary.reasoning
  return { model, reasoning }
}

/**
 * One controller chat per scope. The first open creates the session with a
 * fresh, unguessable id, its model in the metadata the console reads, and a
 * first send that freezes the controller profile and its function policy;
 * later messages from the console chat inherit both. A deleted session is
 * replaced on the next open.
 */
export class ControllerSessions {
  private readonly opening = new Map<string, Promise<{ session_id: string; created: boolean }>>()

  constructor(private readonly deps: ControllerDeps) {}

  open(scope: string): Promise<{ session_id: string; created: boolean }> {
    const running = this.opening.get(scope)
    if (running) return running
    const next = this.openOnce(scope).finally(() => this.opening.delete(scope))
    this.opening.set(scope, next)
    return next
  }

  private async openOnce(scope: string) {
    const { iii } = this.deps
    const key = `controller:${scope}`
    const stored = await iii.call<StoredController | null>('state::get', {
      scope: STATE_SCOPE,
      key,
    })
    if (stored?.session_id) {
      const existing = await iii
        .call<{ meta?: unknown } | null>('session::get', { session_id: stored.session_id })
        .catch(() => null)
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
        ...(reasoning && reasoning !== 'default' && reasoning !== 'lowest'
          ? { thinking_level: reasoning }
          : {}),
      },
    })
    const [provider, ...modelParts] = model.split('::')
    await iii.call('harness::send', {
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
    await iii.call('state::set', {
      scope: STATE_SCOPE,
      key,
      value: {
        session_id: sessionId,
        created_at: new Date().toISOString(),
      } satisfies StoredController,
    })
    return { session_id: sessionId, created: true }
  }
}
