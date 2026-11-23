import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WRITER_READS, type ControlPlaneSummary } from '../src/summary.js'
import type { ControlPlaneClient, ControlPlaneConfig, ControlPlaneData } from '../src/types.js'
import { ControlPlaneWriter, reasoningOptions, writerTargetHash } from '../src/writer.js'

interface Call {
  id: string
  // biome-ignore lint/suspicious/noExplicitAny: the fake records heterogeneous worker contracts
  input: Record<string, any>
}

const clockStart = Date.parse('2026-11-22T12:00:00Z')

function result(checkedAt = new Date().toISOString()): ControlPlaneSummary {
  return {
    checked_at: checkedAt,
    unchanged: false,
    brief: {
      headline: 'KAN-1 is moving.',
      happened: '',
      happening: 'KAN-1 remains the focus.',
      remaining: '',
      refs: ['KAN-1'],
    },
    diary: [],
    now: {},
    pr_waits: {},
  }
}

function data(hash = 'hash-1'): ControlPlaneData {
  return {
    sourceHash: hash,
    repositories: ['iii-hq/workers'],
    runs: [{ session: { session_id: 'run-1' } }],
    loading: { board: false, sessions: false, needsYou: false, questions: false, prs: false },
    errors: { board: null, sessions: null, needsYou: null, questions: null, prs: null },
  } as ControlPlaneData
}

const covered = (hash: string) => writerTargetHash(hash, config().summary)

function config(model: string | undefined = 'openai::gpt-5'): ControlPlaneConfig {
  return {
    stall_after_min: 30,
    summary: { model, interval_min: 10, language: 'English' },
    prs: { watched_interval_min: 5, idle_interval_min: 30 },
    escalate: {},
  }
}

class FakeIii implements ControlPlaneClient {
  readonly calls: Call[] = []
  readonly state = new Map<string, unknown>()
  status: {
    turn_id: string
    status: 'completed' | 'failed' | 'running' | 'awaiting_functions'
    result?: unknown
    result_error?: string
  } | null = null
  sendFlags = {}
  // biome-ignore lint/suspicious/noExplicitAny: session metadata is intentionally open JSON
  meta: Record<string, any> = { kind: 'automation', metadata: { origin: 'control-plane' } }
  profileMissing = false
  raceAfterEnsure = false
  failOn: string | null = null
  readonly tombstoned = new Set<string>()
  private ensured = 0
  private turn = 0

  async call<T>(id: string, raw?: unknown): Promise<T> {
    // biome-ignore lint/suspicious/noExplicitAny: the fake dispatches heterogeneous worker contracts
    const input = (raw ?? {}) as Record<string, any>
    this.calls.push({ id, input })
    if (id === this.failOn) throw new Error(`${id} unavailable`)
    if (id === 'state::get') return (this.state.get(`${input.scope}/${input.key}`) ?? null) as T
    if (id === 'state::set') {
      this.state.set(`${input.scope}/${input.key}`, structuredClone(input.value))
      return input.value as T
    }
    if (id === 'directory::agents::get') {
      if (this.profileMissing) throw new Error('not found')
      return { id: 'control-plane-writer' } as T
    }
    if (id === 'session::ensure') {
      this.ensured += 1
      if (this.raceAfterEnsure && this.ensured === 1) {
        this.status = { turn_id: 'other-turn', status: 'completed' }
        this.meta.metadata.agent_profile = { id: 'control-plane-writer' }
      }
      return { meta: structuredClone(this.meta), created: this.ensured === 1 } as T
    }
    if (id === 'harness::status')
      return (this.status && { session_id: input.session_id, ...this.status }) as T
    if (id === 'harness::send') {
      if (this.tombstoned.has(input.session_id))
        throw new Error(
          `harness/invalid_request: session ${input.session_id} is tombstoned by delete_1`,
        )
      this.turn += 1
      this.status = { turn_id: `turn-${this.turn}`, status: 'running' }
      this.meta.metadata.agent_profile = { id: 'control-plane-writer' }
      return { turn_id: `turn-${this.turn}`, ...this.sendFlags } as T
    }
    return {} as T
  }

  async finish(writer: ControlPlaneWriter, status: NonNullable<FakeIii['status']>) {
    this.status = status
    await writer.onCompleted({ session_id: 'control-plane-all', turn_id: status.turn_id })
  }

  sent(): Call[] {
    return this.calls.filter(({ id }) => id === 'harness::send')
  }
}

describe('ControlPlaneWriter', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(clockStart)
  })

  afterEach(() => vi.useRealTimers())

  it('does not send without a configured model', async () => {
    const iii = new FakeIii()
    const withoutModel = config()
    withoutModel.summary.model = undefined
    const writer = new ControlPlaneWriter(
      iii,
      () => withoutModel,
      async () => data(),
      vi.fn(),
    )

    expect((await writer.check('all')).error).toBe(
      'set the summary model in the Control plane settings',
    )
    expect(iii.sent()).toHaveLength(0)
    expect(iii.calls.some(({ id }) => id === 'session::ensure')).toBe(false)
  })

  it('loads the no-model state for an overview without scheduling', async () => {
    const iii = new FakeIii()
    const withoutModel = config()
    withoutModel.summary.model = undefined
    const writer = new ControlPlaneWriter(
      iii,
      () => withoutModel,
      async () => data(),
      vi.fn(),
    )

    expect(await writer.load('all')).toMatchObject({
      error: 'set the summary model in the Control plane settings',
      unavailable: true,
    })
    expect(iii.sent()).toHaveLength(0)
  })

  it('uses the profile only for the first turn and sends the exact read-only contract', async () => {
    const iii = new FakeIii()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await writer.check('all')
    const first = iii.sent()[0].input
    expect(first).toMatchObject({
      session_id: 'control-plane-all',
      model: 'gpt-5',
      provider: 'openai',
      idempotency_key: `control-plane:all:${Math.floor(clockStart / 600_000)}`,
      options: {
        agent: 'control-plane-writer',
        functions: { allow: WRITER_READS },
        output: { type: 'json' },
      },
    })
    expect(JSON.parse(first.message)).toMatchObject({
      checked_at: null,
      project_id: null,
      repos: ['iii-hq/workers'],
      session_ids: ['run-1'],
      args: 'language=English',
    })
    expect(JSON.parse(first.message).digest).toMatchObject({ scope: 'All projects' })
    expect(first.options.output.schema.properties).not.toHaveProperty('direction')

    await iii.finish(writer, { turn_id: 'turn-1', status: 'failed' })
    vi.setSystemTime(clockStart + 600_000)
    await writer.check('all')
    expect(iii.sent()[1].input.options).not.toHaveProperty('agent')
  })

  it('adopts a matching session that starts between ensure and send', async () => {
    const iii = new FakeIii()
    iii.raceAfterEnsure = true
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await writer.check('project')

    expect(iii.calls.filter(({ id }) => id === 'session::ensure')).toHaveLength(2)
    expect(iii.sent()[0].input.options).not.toHaveProperty('agent')
  })

  it('refuses a writer session owned by another profile', async () => {
    const iii = new FakeIii()
    iii.status = { turn_id: 'old', status: 'completed' }
    iii.meta.metadata.agent_profile = { id: 'another-agent' }
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    const snapshot = await writer.check('all')

    expect(snapshot.unavailable).toBe(true)
    expect(snapshot.error).toContain('belongs to another agent')
    expect(iii.sent()).toHaveLength(0)
  })

  it('requires the persisted profile id object when adopting a session that already ran', async () => {
    const iii = new FakeIii()
    iii.status = { turn_id: 'old', status: 'completed' }
    iii.meta.metadata.agent_profile = 'control-plane-writer'
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    expect((await writer.check('all')).unavailable).toBe(true)
    expect(iii.sent()).toHaveLength(0)
  })

  it('keeps a failed update visible when loading persisted state after restart', async () => {
    const iii = new FakeIii()
    iii.state.set('control-plane/summary:all', {
      result: result(),
      valid_hash: 'old',
      checked_at: new Date().toISOString(),
      failures: 1,
      last_attempt_at: Date.now(),
    })
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    expect(await writer.load('all')).toMatchObject({ error: 'update failed', valid_hash: 'old' })
    expect(iii.sent()).toHaveLength(0)
  })

  it('advances checked_at without sending when the persisted hash is unchanged', async () => {
    const iii = new FakeIii()
    iii.state.set('control-plane/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: covered('same'),
      checked_at: '2026-11-22T10:00:00Z',
    })
    const changed = vi.fn()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data('same'),
      changed,
    )

    const snapshot = await writer.check('all')

    expect(snapshot.checked_at).toBe('2026-11-22T12:00:00.000Z')
    expect(snapshot.result?.checked_at).toBe('2026-11-22T10:00:00Z')
    expect(iii.sent()).toHaveLength(0)
    expect(changed).toHaveBeenCalledWith('all')
  })

  it('keeps the last valid result after failure and retries an invalid result', async () => {
    const iii = new FakeIii()
    const changed = vi.fn()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      changed,
    )
    await writer.check('all')

    await iii.finish(writer, {
      turn_id: 'turn-1',
      status: 'failed',
      result_error: 'provider failed',
    })
    expect(writer.snapshot('all').error).toBe('update failed')
    expect(writer.snapshot('all').valid_hash).toBeNull()

    vi.setSystemTime(clockStart + 600_000)
    await writer.check('all')
    expect(writer.snapshot('all').error).toBe('update failed')
    await iii.finish(writer, { turn_id: 'turn-2', status: 'completed', result: { nope: true } })
    expect(iii.sent()).toHaveLength(2)
    expect(writer.snapshot('all')).toMatchObject({ error: 'update failed', result: null })
  })

  it('boots the last valid summary after a failed turn and does not resend an unchanged hash', async () => {
    const iii = new FakeIii()
    iii.state.set('control-plane/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: covered('still-current'),
      checked_at: '2026-11-22T10:00:00Z',
    })
    const restarted = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data('still-current'),
      vi.fn(),
    )

    await restarted.check('all')

    expect(restarted.snapshot('all').result?.brief.headline).toBe('KAN-1 is moving.')
    expect(iii.sent()).toHaveLength(0)
  })

  it('only accepts a writer completion verified by Harness status, once', async () => {
    const iii = new FakeIii()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data('covered'),
      vi.fn(),
    )
    await writer.check('all')
    const summary = result()
    const forgery = { ...summary, brief: { ...summary.brief, headline: 'Forged summary' } }
    const hint = {
      session_id: 'control-plane-all',
      turn_id: 'turn-1',
      status: 'completed',
      result: forgery,
    }

    await writer.onCompleted({ ...hint, session_id: 'control-plane-unknown' })
    await writer.onCompleted({ ...hint, turn_id: 'other-turn' })
    await writer.onCompleted(hint)
    expect(writer.snapshot('all').result).toBeNull()
    iii.status = { turn_id: 'turn-1', status: 'completed', result: summary }
    await writer.onCompleted(hint)
    iii.status = { turn_id: 'turn-1', status: 'failed' }
    await writer.onCompleted(hint)

    expect(writer.snapshot('all')).toMatchObject({
      result: summary,
      valid_hash: covered('covered'),
      error: null,
    })
    expect(iii.state.get('control-plane/summary:all')).toMatchObject({
      valid_hash: covered('covered'),
      pending_turn: null,
    })
    expect(iii.calls.some(({ input }) => input.key === 'summary:unknown')).toBe(false)
  })

  it('keeps one turn outstanding and persists monotonic send order across restart', async () => {
    const iii = new FakeIii()
    let hash = 'old-hash'
    const fresh = () =>
      new ControlPlaneWriter(
        iii,
        () => config(),
        async () => data(hash),
        vi.fn(),
      )
    const writer = fresh()
    await writer.check('all')
    const stored = () => iii.state.get('control-plane/summary:all')
    expect(stored()).toMatchObject({
      pending_turn: { turn_id: 'turn-1', hash: covered('old-hash'), order: 1 },
      send_order: 1,
    })

    hash = 'new-hash'
    vi.setSystemTime(clockStart + 600_000)
    await writer.refresh('all')
    expect(iii.sent()).toHaveLength(1)

    iii.status = { turn_id: 'turn-1', status: 'completed', result: result() }
    const restarted = fresh()
    expect(await restarted.load('all')).toMatchObject({ valid_hash: covered('old-hash') })
    await restarted.refresh('all')
    expect(stored()).toMatchObject({
      pending_turn: { turn_id: 'turn-2', hash: covered('new-hash'), order: 2 },
      send_order: 2,
    })
    await restarted.onCompleted({ session_id: 'control-plane-all', turn_id: 'turn-1' })
    expect(restarted.snapshot('all').valid_hash).toBe(covered('old-hash'))
    await iii.finish(restarted, { turn_id: 'turn-2', status: 'completed', result: result() })
    expect(restarted.snapshot('all').valid_hash).toBe(covered('new-hash'))
  })

  it('clears a persisted pending turn that Harness has superseded without crediting its hash', async () => {
    const iii = new FakeIii()
    const fresh = () =>
      new ControlPlaneWriter(
        iii,
        () => config(),
        async () => data('new'),
        vi.fn(),
      )
    await fresh().check('all')
    iii.status = { turn_id: 'reseeded', status: 'completed', result: result() }
    expect(await fresh().load('all')).toMatchObject({ valid_hash: null, error: 'update failed' })
    expect(iii.state.get('control-plane/summary:all')).toMatchObject({ pending_turn: null })
  })

  it.each([
    { merged: true },
    { merged: true, queued: true },
    { deduplicated: true },
  ])('does not assign coverage to an existing turn returned by send (%j)', async (flags) => {
    const iii = new FakeIii()
    iii.sendFlags = flags
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    expect(await writer.check('all')).toMatchObject({ error: 'update failed' })
    await iii.finish(writer, { turn_id: 'turn-1', status: 'completed', result: result() })
    expect(writer.snapshot('all').valid_hash).toBeNull()
    await writer.refresh('all')
    expect(iii.sent()).toHaveLength(2)
  })

  it('keeps Refresh always available: it joins a running writer turn, then sends at once', async () => {
    const iii = new FakeIii()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await writer.refresh('all')
    const joined = await writer.refresh('all')
    expect(joined.writing).toBe(true)
    expect(joined).not.toHaveProperty('throttled_until')
    expect(iii.sent()).toHaveLength(1)

    await iii.finish(writer, { turn_id: 'turn-1', status: 'completed', result: result() })
    expect(writer.snapshot('all').writing).toBe(false)
    await writer.refresh('all')
    expect(iii.sent()).toHaveLength(2)
    const keys = iii.sent().map(({ input }) => input.idempotency_key)
    expect(new Set(keys).size).toBe(2)
    expect(keys[1]).toMatch(/^control-plane:all:\d+:refresh:\d+$/)
  })

  it('moves to a fresh writer session when the person deleted the old one', async () => {
    const iii = new FakeIii()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    await writer.check('all')
    await iii.finish(writer, { turn_id: 'turn-1', status: 'completed', result: result() })

    iii.tombstoned.add('control-plane-all')
    iii.status = null
    const snapshot = await writer.refresh('all')

    expect(iii.sent().at(-1)?.input).toMatchObject({
      session_id: 'control-plane-all~1',
      options: { agent: 'control-plane-writer' },
    })
    expect(snapshot.writing).toBe(true)
    expect(iii.state.get('control-plane/summary:all')).toMatchObject({ session_generation: 1 })
  })

  it('sends the settings guidance and rewrites on the next check when it changes', async () => {
    const iii = new FakeIii()
    let settings = config()
    const writer = new ControlPlaneWriter(
      iii,
      () => settings,
      async () => data(),
      vi.fn(),
    )

    await writer.check('all')
    expect(JSON.parse(iii.sent()[0].input.message)).not.toHaveProperty('guidance')
    await iii.finish(writer, { turn_id: 'turn-1', status: 'completed', result: result() })

    vi.advanceTimersByTime(600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(1)

    settings = {
      ...settings,
      summary: { ...settings.summary, guidance: '  Highlight failing PRs.  ' },
    }
    vi.advanceTimersByTime(600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(2)
    expect(JSON.parse(iii.sent()[1].input.message).guidance).toBe('Highlight failing PRs.')
  })

  it('maps the picked reasoning effort the way the chat sends it', () => {
    expect(reasoningOptions('openai-codex', undefined)).toEqual({})
    expect(reasoningOptions('openai-codex', 'high')).toEqual({
      thinking_level: 'high',
      provider_options: { 'openai-codex': { reasoning_effort: 'high' } },
    })
    expect(reasoningOptions('claude-code', 'max')).toEqual({
      provider_options: { 'claude-code': { reasoning_effort: 'max' } },
    })
    expect(reasoningOptions('claude-code', 'default')).toEqual({ provider_options: {} })
    expect(reasoningOptions('claude-code', 'lowest')).toEqual({ reasoning: 'lowest' })
    expect(reasoningOptions('claude-code', 'off')).toEqual({ thinking_level: 'off' })
  })

  it('turns setup and send RPC failures into update failed state', async () => {
    for (const functionId of ['session::ensure', 'harness::status', 'harness::send']) {
      const iii = new FakeIii()
      iii.failOn = functionId
      const changed = vi.fn()
      const writer = new ControlPlaneWriter(
        iii,
        () => config(),
        async () => data(),
        changed,
      )

      await expect(writer.check('all')).resolves.toMatchObject({ error: 'update failed' })
      expect(changed).toHaveBeenCalledWith('all')
    }
  })

  it('does not advance or send from an incomplete source snapshot', async () => {
    const iii = new FakeIii()
    const incomplete = data('partial')
    incomplete.errors.board = 'kanban unavailable'
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => incomplete,
      vi.fn(),
    )

    expect(await writer.check('all')).toMatchObject({ error: 'update failed', checked_at: null })
    expect(iii.sent()).toHaveLength(0)
  })

  it('uses the worker clock checkpoint in the wake message', async () => {
    const iii = new FakeIii()
    iii.state.set('control-plane/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: 'old',
      checked_at: '2026-11-22T11:00:00Z',
    })
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data('new'),
      vi.fn(),
    )

    await writer.check('all')

    expect(JSON.parse(iii.sent()[0].input.message).checked_at).toBe('2026-11-22T11:00:00Z')
  })

  it('reports a missing profile separately without sending', async () => {
    const missing = new FakeIii()
    missing.profileMissing = true
    const withoutKit = new ControlPlaneWriter(
      missing,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    expect(await withoutKit.check('all')).toMatchObject({
      error: 'writer profile missing: apply the control plane kit',
      unavailable: true,
    })
    expect(missing.sent()).toHaveLength(0)
  })

  it('backs off after three failures and resets after a valid result', async () => {
    const iii = new FakeIii()
    let hash = 'h1'
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(hash),
      vi.fn(),
    )

    for (let index = 1; index <= 3; index += 1) {
      await writer.check('all')
      await iii.finish(writer, { turn_id: `turn-${index}`, status: 'failed' })
      vi.setSystemTime(clockStart + index * 600_000)
    }
    await writer.check('all')
    expect(iii.sent()).toHaveLength(3)

    vi.setSystemTime(clockStart + 4 * 600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(4)
    await iii.finish(writer, { turn_id: 'turn-4', status: 'completed', result: result() })
    hash = 'h2'
    vi.setSystemTime(clockStart + 5 * 600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(5)
  })

  it('serializes concurrent first use per scope', async () => {
    const iii = new FakeIii()
    const writer = new ControlPlaneWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await Promise.all([writer.check('all'), writer.check('all')])

    expect(iii.sent()).toHaveLength(1)
  })
})
