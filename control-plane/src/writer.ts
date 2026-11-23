import { type ActivityRow, writerDigest } from './digest.js'
import { acceptSummary, SUMMARY_SCHEMA, type ControlPlaneSummary, WRITER_READS } from './summary.js'
import type { ControlPlaneClient, ControlPlaneConfig, ControlPlaneData } from './types.js'

const STATE_SCOPE = 'control-plane'
const PROFILE_ERROR = 'writer profile missing: apply the control plane kit'
const MODEL_ERROR = 'set the summary model in the Control plane settings'
const UPDATE_ERROR = 'update failed'

interface StoredSummary {
  result: ControlPlaneSummary | null
  valid_hash: string | null
  checked_at: string | null
  pending_turn?: PendingTurn | null
  send_order?: number
  failures?: number
  last_attempt_at?: number
  last_send_at?: number
  /** Bumped when the person deletes the writer session; the harness never reuses a deleted id. */
  session_generation?: number
  // The previous format had separate clocks for scheduled checks and Refresh.
  last_refresh_at?: number
  pending_turns?: Record<string, { sent_at: number }>
}

interface PendingTurn {
  turn_id: string
  hash: string
  order: number
}

interface WriterState extends StoredSummary {
  error: string | null
  unavailable: boolean
  failures: number
  lastAttemptAt: number
  lastSendAt: number
  sendOrder: number
  generation: number
  pending: PendingTurn | null
  booted: boolean
}

export interface WriterSnapshot
  extends Pick<StoredSummary, 'result' | 'valid_hash' | 'checked_at'> {
  error: string | null
  unavailable: boolean
  /** A writer turn is running for this scope; Refresh joins it instead of starting another. */
  writing: boolean
}

export type RefreshResult = WriterSnapshot

interface SessionStatus {
  session_id: string
  turn_id?: string | null
  status: 'running' | 'awaiting_functions' | 'completed' | 'cancelled' | 'failed'
  queued?: unknown[]
  result?: unknown
  result_error?: string | null
}

const terminal = (status: SessionStatus) =>
  status.status === 'completed' || status.status === 'cancelled' || status.status === 'failed'

interface SessionEnsure {
  meta: {
    kind?: string
    metadata?: Record<string, unknown> | null
  }
}

function initialState(): WriterState {
  return {
    result: null,
    valid_hash: null,
    checked_at: null,
    error: null,
    unavailable: false,
    failures: 0,
    lastAttemptAt: 0,
    lastSendAt: 0,
    sendOrder: 0,
    generation: 0,
    pending: null,
    booted: false,
  }
}

/** What a valid result covers: the sources plus the person's prose preferences. */
export function writerTargetHash(
  sourceHash: string,
  summary: ControlPlaneConfig['summary'],
): string {
  const { model, reasoning, language, guidance } = summary
  return `${sourceHash}|${JSON.stringify([model ?? null, reasoning ?? null, language, guidance?.trim() ?? ''])}`
}

const THINKING_LEVELS = ['off', 'minimal', 'low', 'medium', 'high', 'xhigh']

/** The chat's mapping of a picked effort onto `harness::send` options (ade/web `lib/backend/real.ts`). */
export function reasoningOptions(
  provider: string,
  effort: string | undefined,
): Record<string, unknown> {
  if (!effort) return {}
  if (effort === 'lowest') return { reasoning: 'lowest' }
  return {
    ...(THINKING_LEVELS.includes(effort) ? { thinking_level: effort } : {}),
    ...(effort === 'off'
      ? {}
      : {
          provider_options:
            effort === 'default' ? {} : { [provider]: { reasoning_effort: effort } },
        }),
  }
}

function publicState(state: WriterState): WriterSnapshot {
  return {
    result: state.result,
    valid_hash: state.valid_hash,
    checked_at: state.checked_at,
    error: state.error,
    unavailable: state.unavailable,
    writing: Boolean(state.pending),
  }
}

function profileId(metadata: Record<string, unknown> | null | undefined): string | undefined {
  const profile = metadata?.agent_profile
  if (profile && typeof profile === 'object' && 'id' in profile) {
    const id = profile.id
    if (typeof id === 'string') return id
  }
  return undefined
}

export class ControlPlaneWriter {
  private readonly states = new Map<string, WriterState>()
  private readonly locks = new Map<string, Promise<unknown>>()
  private stopped = false

  constructor(
    private readonly iii: ControlPlaneClient,
    private readonly config: () => ControlPlaneConfig,
    private readonly sources: (scope: string) => Promise<ControlPlaneData>,
    private readonly changed: (scope: string) => Promise<void> | void,
  ) {}

  snapshot(scope: string): WriterSnapshot {
    return publicState(this.applyConfiguration(this.states.get(scope) ?? initialState()))
  }

  async load(scope: string): Promise<WriterSnapshot> {
    return this.lock(scope, async () =>
      publicState(this.applyConfiguration(await this.boot(scope))),
    )
  }

  nextCheckAt(scope: string): number {
    const state = this.states.get(scope)
    if (!state?.lastAttemptAt) return 0
    const backoff = state.failures < 3 ? 1 : Math.min(8, 2 ** (state.failures - 2))
    const interval = this.interval()
    return Math.max(state.lastAttemptAt + interval * backoff, state.lastSendAt + interval)
  }

  async check(scope: string): Promise<WriterSnapshot> {
    return this.update(scope, false)
  }

  async refresh(scope: string): Promise<RefreshResult> {
    return this.update(scope, true)
  }

  private async update(scope: string, refresh: boolean): Promise<RefreshResult> {
    return this.lock(scope, async () => {
      const state = this.applyConfiguration(await this.boot(scope))
      if (this.stopped || !this.config().summary.model) return publicState(state)

      const now = Date.now()
      const nextSend = state.lastSendAt + this.interval()
      // Refresh is always available. It sends at once, unless a writer turn is
      // already running: then it joins that turn. One outstanding turn per
      // scope is what bounds the spend.
      if (state.pending && (refresh || now >= this.nextCheckAt(scope))) {
        await this.reconcile(scope, state, await this.status(scope))
      }
      if (state.pending || (!refresh && now < this.nextCheckAt(scope))) {
        if (state.pending && now >= this.nextCheckAt(scope)) state.lastAttemptAt = now
        return publicState(state)
      }
      state.lastAttemptAt = now
      try {
        const data = await this.sources(scope)
        if (this.sourceUnavailable(data)) {
          await this.fail(scope, state)
        } else if (!refresh && this.targetHash(data) === state.valid_hash) {
          state.checked_at = new Date().toISOString()
          state.error = null
          state.unavailable = false
          state.failures = 0
          await this.persist(scope, state)
          await this.changed(scope)
        } else if (refresh || !state.lastSendAt || Date.now() >= nextSend) {
          await this.sendFresh(scope, state, data, refresh)
        }
      } catch (error) {
        console.error(`control-plane writer ${scope}: update failed`, error)
        await this.fail(scope, state)
      }
      return publicState(state)
    })
  }

  async onCompleted(hint: unknown): Promise<void> {
    if (
      !hint ||
      typeof hint !== 'object' ||
      !('session_id' in hint) ||
      typeof hint.session_id !== 'string' ||
      !hint.session_id.startsWith('control-plane-') ||
      !('turn_id' in hint) ||
      typeof hint.turn_id !== 'string'
    )
      return

    const scope = hint.session_id.slice('control-plane-'.length).replace(/~\d+$/, '')
    // A callable sink cannot create scope state or supply a result.
    if (!this.states.has(scope)) return
    await this.lock(scope, async () => {
      const state = this.states.get(scope)
      if (!state?.pending || state.pending.turn_id !== hint.turn_id) return
      const status = await this.status(scope)
      if (status?.turn_id === hint.turn_id) await this.reconcile(scope, state, status)
    })
  }

  private async reconcile(scope: string, state: WriterState, status: SessionStatus | null) {
    const pending = state.pending
    if (!pending) return
    const matches =
      status?.session_id === this.sessionId(scope) && status.turn_id === pending.turn_id
    if (matches && !terminal(status)) return
    state.pending = null
    const next =
      matches &&
      pending.order === state.sendOrder &&
      status.status === 'completed' &&
      !status.result_error
        ? acceptSummary(status.result, state.result)
        : null
    if (!next || next === state.result) {
      await this.fail(scope, state)
      return
    }
    state.result = next
    state.valid_hash = pending.hash
    state.checked_at = new Date().toISOString()
    state.error = null
    state.unavailable = false
    state.failures = 0
    await this.persist(scope, state)
    await this.changed(scope)
  }

  private interval(): number {
    return this.config().summary.interval_min * 60_000
  }

  private sessionId(scope: string): string {
    const generation = this.state(scope).generation
    return `control-plane-${scope}${generation ? `~${generation}` : ''}`
  }

  private status(scope: string): Promise<SessionStatus | null> {
    return this.iii.call('harness::status', { session_id: this.sessionId(scope), verbose: true })
  }

  /**
   * The harness never reuses a deleted session id. When the person deleted the
   * writer session, the scope moves to the next one and the send runs again there.
   */
  private async sendFresh(
    scope: string,
    state: WriterState,
    data: ControlPlaneData,
    refresh: boolean,
  ) {
    try {
      await this.send(scope, state, data, refresh)
    } catch (error) {
      if (!String(error).includes('is tombstoned by')) throw error
      state.generation += 1
      await this.send(scope, state, data, refresh)
    }
  }

  dispose(): void {
    this.stopped = true
    this.locks.clear()
  }

  private state(scope: string): WriterState {
    let state = this.states.get(scope)
    if (!state) {
      state = initialState()
      this.states.set(scope, state)
    }
    return state
  }

  private async boot(scope: string): Promise<WriterState> {
    const state = this.state(scope)
    if (state.booted) return state
    const stored = await this.iii.call<StoredSummary | null>('state::get', {
      scope: STATE_SCOPE,
      key: `summary:${scope}`,
    })
    if (stored && typeof stored === 'object') {
      // A result stored in an older shape is dropped; the next check rewrites it.
      state.result = stored.result ? acceptSummary(stored.result, null) : null
      state.valid_hash = stored.valid_hash ?? null
      state.checked_at = stored.checked_at ?? null
      state.pending = stored.pending_turn ?? null
      state.sendOrder = stored.send_order ?? 0
      state.generation = stored.session_generation ?? 0
      state.failures = stored.failures ?? 0
      if (state.failures > 0) state.error = UPDATE_ERROR
      state.lastAttemptAt = stored.last_attempt_at ?? 0
      state.lastSendAt =
        stored.last_send_at ??
        Math.max(
          stored.last_refresh_at ?? 0,
          state.lastAttemptAt,
          ...Object.values(stored.pending_turns ?? {}).map(({ sent_at }) => sent_at),
        )
      if (state.pending) await this.reconcile(scope, state, await this.status(scope))
    }
    state.booted = true
    return state
  }

  private async persist(scope: string, state: WriterState): Promise<void> {
    await this.iii.call('state::set', {
      scope: STATE_SCOPE,
      key: `summary:${scope}`,
      value: {
        result: state.result,
        valid_hash: state.valid_hash,
        checked_at: state.checked_at,
        pending_turn: state.pending,
        send_order: state.sendOrder,
        session_generation: state.generation,
        failures: state.failures,
        last_attempt_at: state.lastAttemptAt,
        last_send_at: state.lastSendAt,
      } satisfies StoredSummary,
    })
  }

  private async fail(scope: string, state: WriterState): Promise<void> {
    state.failures += 1
    state.error = UPDATE_ERROR
    state.unavailable = false
    await this.persist(scope, state).catch(() => undefined)
    await this.changed(scope)
  }

  private async send(
    scope: string,
    state: WriterState,
    data: ControlPlaneData,
    refresh: boolean,
  ): Promise<void> {
    const config = this.config()
    const model = config.summary.model
    if (!model) return
    try {
      await this.iii.call('directory::agents::get', { id: 'control-plane-writer' })
    } catch {
      state.error = PROFILE_ERROR
      state.unavailable = true
      await this.changed(scope)
      return
    }

    const sessionId = this.sessionId(scope)
    let ensured = await this.iii.call<SessionEnsure>('session::ensure', {
      session_id: sessionId,
      title: sessionId,
      kind: 'automation',
      metadata: { origin: 'control-plane' },
    })
    const status = await this.status(scope)
    if (status?.turn_id) {
      ensured = await this.iii.call<SessionEnsure>('session::ensure', { session_id: sessionId })
    }
    const metadata = ensured.meta.metadata
    if (
      ensured.meta.kind !== 'automation' ||
      metadata?.origin !== 'control-plane' ||
      (status?.turn_id && profileId(metadata) !== 'control-plane-writer')
    ) {
      state.error = 'writer session unavailable: belongs to another agent or origin'
      state.unavailable = true
      await this.changed(scope)
      return
    }
    if (status && (!terminal(status) || status.queued?.length)) return

    state.lastSendAt = Date.now()
    state.sendOrder += 1
    await this.persist(scope, state)
    const slot = Math.floor(state.lastSendAt / this.interval())
    const [provider, ...modelParts] = model.split('::')
    const activity = await this.iii
      .call<{ activity?: ActivityRow[] }>('kanban::activity::list', {
        since: state.checked_at ?? new Date(Date.now() - 86_400_000).toISOString(),
        limit: 200,
      })
      .then((result) => result?.activity ?? [])
      .catch(() => [])
    const digest = writerDigest(data, {
      scopeName:
        scope === 'all'
          ? 'All projects'
          : ((data.projects ?? []).find((project) => project.id === scope)?.name ?? scope),
      since: state.checked_at,
      activity,
    })
    const sent = await this.iii.call<{
      turn_id?: string
      merged?: boolean
      queued?: boolean
      deduplicated?: boolean
    }>('harness::send', {
      session_id: sessionId,
      idempotency_key: `control-plane:${scope}:${slot}${refresh ? `:refresh:${state.sendOrder}` : ''}`,
      message: JSON.stringify({
        checked_at: state.checked_at,
        refresh_at: new Date().toISOString(),
        project_id: scope === 'all' ? null : scope,
        repos: data.repositories,
        session_ids: data.runs.map((run) => run.session.session_id),
        args: `language=${config.summary.language}`,
        ...(config.summary.guidance?.trim() ? { guidance: config.summary.guidance.trim() } : {}),
        digest,
      }),
      provider,
      model: modelParts.join('::'),
      options: {
        ...(!status?.turn_id ? { agent: 'control-plane-writer' } : {}),
        ...reasoningOptions(provider, config.summary.reasoning),
        functions: { allow: WRITER_READS },
        output: { type: 'json', schema: SUMMARY_SCHEMA },
      },
    })
    // A merged, queued or deduplicated send ID does not identify a new turn
    // that covers this snapshot. The attempt still consumes the reservation.
    if (!sent.turn_id || sent.merged || sent.queued || sent.deduplicated) {
      console.error(`control-plane writer ${scope}: the send did not start a turn`, sent)
      await this.fail(scope, state)
      return
    }
    state.pending = { turn_id: sent.turn_id, hash: this.targetHash(data), order: state.sendOrder }
    await this.persist(scope, state)
    if (state.error !== UPDATE_ERROR) state.error = null
    state.unavailable = false
  }

  /**
   * What a valid result covers: the sources, and the person's prose preferences.
   * Changing the model, effort, language or guidance makes the next check rewrite.
   */
  private targetHash(data: ControlPlaneData): string {
    return writerTargetHash(data.sourceHash, this.config().summary)
  }

  private applyConfiguration(state: WriterState): WriterState {
    if (!this.config().summary.model) {
      state.error = MODEL_ERROR
      state.unavailable = true
    } else if (state.error === MODEL_ERROR) {
      state.error = null
      state.unavailable = false
    }
    return state
  }

  private sourceUnavailable(data: ControlPlaneData): boolean {
    return (
      Object.values(data.loading).some(Boolean) ||
      Object.values(data.errors).some((error) => error !== null)
    )
  }

  private lock<T>(scope: string, work: () => Promise<T>): Promise<T> {
    const previous = this.locks.get(scope) ?? Promise.resolve()
    const current = previous.catch(() => undefined).then(work)
    this.locks.set(scope, current)
    const cleanup = () => {
      if (this.locks.get(scope) === current) this.locks.delete(scope)
    }
    void current.then(cleanup, cleanup)
    return current
  }
}
