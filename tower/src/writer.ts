import { acceptSummary, SUMMARY_SCHEMA, type TowerSummary, WRITER_READS } from './summary.js'
import type { TowerClient, TowerConfig, TowerData } from './types.js'

const STATE_SCOPE = 'tower'
const PROFILE_ERROR = 'writer profile missing: apply the tower kit'
const MODEL_ERROR = 'set the summary model in the tower settings'
const UPDATE_ERROR = 'update failed'

interface StoredSummary {
  result: TowerSummary | null
  valid_hash: string | null
  checked_at: string | null
  pending_turns?: Record<string, { hash: string; sent_at: number }>
  covered_at?: number
  failures?: number
  last_attempt_at?: number
  last_refresh_at?: number
}

interface WriterState extends StoredSummary {
  error: string | null
  unavailable: boolean
  failures: number
  lastAttemptAt: number
  lastRefreshAt: number
  coveredAt: number
  pending: Record<string, { hash: string; sent_at: number }>
  booted: boolean
}

export interface WriterSnapshot extends StoredSummary {
  error: string | null
  unavailable: boolean
}

export interface RefreshResult extends WriterSnapshot {
  throttled_until?: string
}

export interface TurnCompleted {
  session_id: string
  turn_id: string
  status: string
  terminal?: boolean
  result?: unknown
  result_error?: string | null
}

interface SessionStatus {
  turn_id?: string | null
}

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
    lastRefreshAt: 0,
    coveredAt: 0,
    pending: {},
    booted: false,
  }
}

function publicState(state: WriterState): WriterSnapshot {
  return {
    result: state.result,
    valid_hash: state.valid_hash,
    checked_at: state.checked_at,
    error: state.error,
    unavailable: state.unavailable,
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

export class TowerWriter {
  private readonly states = new Map<string, WriterState>()
  private readonly locks = new Map<string, Promise<unknown>>()
  private readonly approvalsConfigured = new Set<string>()
  private approvalAvailable: Promise<boolean> | undefined
  private stopped = false

  constructor(
    private readonly iii: TowerClient,
    private readonly config: () => TowerConfig,
    private readonly sources: (scope: string) => Promise<TowerData>,
    private readonly changed: (scope: string) => Promise<void> | void,
  ) {}

  snapshot(scope: string): WriterSnapshot {
    return publicState(this.applyConfiguration(this.state(scope)))
  }

  async load(scope: string): Promise<WriterSnapshot> {
    return this.lock(scope, async () =>
      publicState(this.applyConfiguration(await this.boot(scope))),
    )
  }

  invalidateAvailability(): void {
    this.approvalAvailable = undefined
    this.approvalsConfigured.clear()
  }

  async check(scope: string, force = false): Promise<WriterSnapshot> {
    return this.lock(scope, async () => {
      const state = await this.boot(scope)
      if (this.stopped) return publicState(state)
      const config = this.config()
      if (!config.summary.model) {
        state.error = MODEL_ERROR
        state.unavailable = true
        return publicState(state)
      }
      const interval = config.summary.interval_min * 60_000
      const backoff = state.failures < 3 ? 1 : Math.min(8, 2 ** (state.failures - 2))
      if (!force && Date.now() - state.lastAttemptAt < interval * backoff) {
        return publicState(state)
      }
      const data = await this.sources(scope)
      state.lastAttemptAt = Date.now()
      if (this.sourceUnavailable(data)) {
        await this.fail(scope, state)
        return publicState(state)
      }
      if (data.sourceHash === state.valid_hash) {
        state.checked_at = new Date().toISOString()
        state.error = null
        state.unavailable = false
        await this.persist(scope, state)
        await this.changed(scope)
        return publicState(state)
      }
      try {
        await this.send(scope, data, false)
      } catch {
        await this.fail(scope, state)
      }
      return publicState(state)
    })
  }

  async refresh(scope: string): Promise<RefreshResult> {
    return this.lock(scope, async () => {
      const state = await this.boot(scope)
      const interval = this.config().summary.interval_min * 60_000
      if (Date.now() - state.lastRefreshAt < interval) {
        return {
          ...publicState(state),
          throttled_until: new Date(state.lastRefreshAt + interval).toISOString(),
        }
      }
      if (!this.config().summary.model) {
        state.error = MODEL_ERROR
        state.unavailable = true
        return publicState(state)
      }
      const data = await this.sources(scope)
      state.lastRefreshAt = Date.now()
      state.lastAttemptAt = Date.now()
      if (this.sourceUnavailable(data)) {
        await this.fail(scope, state)
        return publicState(state)
      }
      try {
        await this.send(scope, data, true)
      } catch {
        await this.fail(scope, state)
      }
      return publicState(state)
    })
  }

  async onCompleted(event: TurnCompleted): Promise<void> {
    if (!event.session_id.startsWith('tower-') || event.terminal === false) return
    const scope = event.session_id.slice('tower-'.length)
    await this.lock(scope, async () => {
      const state = await this.boot(scope)
      const pending = state.pending[event.turn_id]
      if (!pending) return
      delete state.pending[event.turn_id]
      if (pending.sent_at < state.coveredAt) {
        await this.persist(scope, state)
        return
      }
      if (event.status !== 'completed' || event.result_error || event.result === undefined) {
        await this.fail(scope, state)
        return
      }
      const next = acceptSummary(event.result, state.result)
      if (!next || next === state.result) {
        await this.fail(scope, state)
        return
      }
      state.result = next
      state.valid_hash = pending.hash
      state.coveredAt = pending.sent_at
      state.checked_at = new Date().toISOString()
      state.error = null
      state.unavailable = false
      state.failures = 0
      await this.persist(scope, state)
      await this.changed(scope)
    })
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
      state.result = stored.result ?? null
      state.valid_hash = stored.valid_hash ?? null
      state.checked_at = stored.checked_at ?? null
      state.pending = stored.pending_turns ?? {}
      state.coveredAt = stored.covered_at ?? 0
      state.failures = stored.failures ?? 0
      if (state.failures > 0) state.error = UPDATE_ERROR
      state.lastAttemptAt = stored.last_attempt_at ?? 0
      state.lastRefreshAt = stored.last_refresh_at ?? 0
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
        pending_turns: state.pending,
        covered_at: state.coveredAt,
        failures: state.failures,
        last_attempt_at: state.lastAttemptAt,
        last_refresh_at: state.lastRefreshAt,
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

  private async send(scope: string, data: TowerData, refresh: boolean): Promise<void> {
    const state = this.state(scope)
    const config = this.config()
    const model = config.summary.model
    if (!model) return
    try {
      await this.iii.call('directory::agents::get', { id: 'tower-writer' })
    } catch {
      state.error = PROFILE_ERROR
      state.unavailable = true
      await this.changed(scope)
      return
    }

    const sessionId = `tower-${scope}`
    let ensured = await this.iii.call<SessionEnsure>('session::ensure', {
      session_id: sessionId,
      title: sessionId,
      kind: 'automation',
      metadata: { origin: 'tower' },
    })
    const status = await this.iii.call<SessionStatus | null>('harness::status', {
      session_id: sessionId,
      verbose: true,
    })
    if (status?.turn_id) {
      ensured = await this.iii.call<SessionEnsure>('session::ensure', { session_id: sessionId })
    }
    const metadata = ensured.meta.metadata
    if (
      ensured.meta.kind !== 'automation' ||
      metadata?.origin !== 'tower' ||
      (status?.turn_id && profileId(metadata) !== 'tower-writer')
    ) {
      state.error = 'writer session unavailable: belongs to another agent or origin'
      state.unavailable = true
      await this.changed(scope)
      return
    }
    await this.configureApprovals(sessionId)

    const interval = config.summary.interval_min * 60_000
    const slot = Math.floor(Date.now() / interval)
    const [provider, ...modelParts] = model.split('::')
    const sent = await this.iii.call<{ turn_id?: string }>('harness::send', {
      session_id: sessionId,
      idempotency_key: `tower:${scope}:${slot}${refresh ? ':refresh' : ''}`,
      message: JSON.stringify({
        checked_at: state.checked_at,
        refresh_at: new Date().toISOString(),
        project_id: scope === 'all' ? null : scope,
        repos: data.repositories,
        session_ids: data.runs.map((run) => run.session.session_id),
        args: `language=${config.summary.language}`,
      }),
      provider,
      model: modelParts.join('::'),
      options: {
        ...(!status?.turn_id ? { agent: 'tower-writer' } : {}),
        functions: { allow: WRITER_READS },
        output: { type: 'json', schema: SUMMARY_SCHEMA },
      },
    })
    if (sent.turn_id) {
      state.pending[sent.turn_id] = { hash: data.sourceHash, sent_at: Date.now() }
      await this.persist(scope, state)
    }
    if (state.error !== UPDATE_ERROR) state.error = null
    state.unavailable = false
  }

  private async configureApprovals(sessionId: string): Promise<void> {
    if (this.approvalsConfigured.has(sessionId) || !(await this.hasApprovalGate())) return
    await this.iii.call('approval::set-mode', { session_id: sessionId, mode: 'auto' })
    for (const functionId of WRITER_READS) {
      await this.iii.call('approval::add-always-allow', {
        session_id: sessionId,
        function_id: functionId,
      })
    }
    this.approvalsConfigured.add(sessionId)
  }

  private hasApprovalGate(): Promise<boolean> {
    if (this.approvalAvailable) return this.approvalAvailable
    let guarded: Promise<boolean>
    const discovered = this.iii
      .call<{ functions?: Array<{ function_id?: string }> }>('engine::functions::list', {
        include_internal: true,
      })
      .then((result) =>
        (result.functions ?? []).some(({ function_id }) => function_id === 'approval::set-mode'),
      )
    guarded = discovered.catch((error) => {
      if (this.approvalAvailable === guarded) this.approvalAvailable = undefined
      throw error
    })
    this.approvalAvailable = guarded
    return guarded
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

  private sourceUnavailable(data: TowerData): boolean {
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
