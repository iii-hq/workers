import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { WRITER_READS, type TowerSummary } from '../src/summary.js'
import type { TowerClient, TowerConfig, TowerData } from '../src/types.js'
import { TowerWriter } from '../src/writer.js'

interface Call {
  id: string
  // biome-ignore lint/suspicious/noExplicitAny: the fake records heterogeneous worker contracts
  input: Record<string, any>
}

const clockStart = Date.parse('2026-11-22T12:00:00Z')

function result(checkedAt = new Date().toISOString()): TowerSummary {
  return {
    checked_at: checkedAt,
    unchanged: false,
    brief: { headline: 'KAN-1 is moving.', text: 'KAN-1 remains the focus.', refs: ['KAN-1'] },
    direction: [{ release_id: 'r1', text: 'R1 follows KAN-1.', refs: ['KAN-1'] }],
    diary: [],
    now: {},
    pr_waits: {},
  }
}

function data(hash = 'hash-1'): TowerData {
  return {
    sourceHash: hash,
    repositories: ['iii-hq/workers'],
    runs: [{ session: { session_id: 'run-1' } }],
    loading: { board: false, sessions: false, needsYou: false, questions: false, prs: false },
    errors: { board: null, sessions: null, needsYou: null, questions: null, prs: null },
  } as TowerData
}

function config(model: string | undefined = 'openai::gpt-5'): TowerConfig {
  return {
    stall_after_min: 30,
    summary: { model, interval_min: 10, language: 'English' },
    prs: { watched_interval_min: 5, idle_interval_min: 30 },
    escalate: {},
    at_risk_days: 3,
  }
}

class FakeIii implements TowerClient {
  readonly calls: Call[] = []
  readonly state = new Map<string, unknown>()
  status: { turn_id?: string | null } | null = null
  // biome-ignore lint/suspicious/noExplicitAny: session metadata is intentionally open JSON
  meta: Record<string, any> = { kind: 'automation', metadata: { origin: 'tower' } }
  profileMissing = false
  approval = true
  raceAfterEnsure = false
  failOn: string | null = null
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
      return { id: 'tower-writer' } as T
    }
    if (id === 'session::ensure') {
      this.ensured += 1
      if (this.raceAfterEnsure && this.ensured === 1) {
        this.status = { turn_id: 'other-turn' }
        this.meta.metadata.agent_profile = { id: 'tower-writer' }
      }
      return { meta: structuredClone(this.meta), created: this.ensured === 1 } as T
    }
    if (id === 'harness::status') return this.status as T
    if (id === 'engine::functions::list') {
      return {
        functions: this.approval ? [{ function_id: 'approval::set-mode' }] : [],
      } as T
    }
    if (id === 'harness::send') {
      this.turn += 1
      this.status = { turn_id: `turn-${this.turn}` }
      this.meta.metadata.agent_profile = { id: 'tower-writer' }
      return { turn_id: `turn-${this.turn}` } as T
    }
    return {} as T
  }

  sent(): Call[] {
    return this.calls.filter(({ id }) => id === 'harness::send')
  }
}

describe('TowerWriter', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(clockStart)
  })

  afterEach(() => vi.useRealTimers())

  it('does not send without a configured model', async () => {
    const iii = new FakeIii()
    const withoutModel = config()
    withoutModel.summary.model = undefined
    const writer = new TowerWriter(
      iii,
      () => withoutModel,
      async () => data(),
      vi.fn(),
    )

    expect((await writer.check('all')).error).toBe('set the summary model in the tower settings')
    expect(iii.sent()).toHaveLength(0)
    expect(iii.calls.some(({ id }) => id === 'session::ensure')).toBe(false)
  })

  it('loads the no-model state for an overview without scheduling', async () => {
    const iii = new FakeIii()
    const withoutModel = config()
    withoutModel.summary.model = undefined
    const writer = new TowerWriter(
      iii,
      () => withoutModel,
      async () => data(),
      vi.fn(),
    )

    expect(await writer.load('all')).toMatchObject({
      error: 'set the summary model in the tower settings',
      unavailable: true,
    })
    expect(iii.sent()).toHaveLength(0)
  })

  it('uses the profile only for the first turn and sends the exact read-only contract', async () => {
    const iii = new FakeIii()
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await writer.check('all')
    const first = iii.sent()[0].input
    expect(first).toMatchObject({
      session_id: 'tower-all',
      model: 'gpt-5',
      provider: 'openai',
      idempotency_key: `tower:all:${Math.floor(clockStart / 600_000)}`,
      options: {
        agent: 'tower-writer',
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
    expect(first.options.output.schema.properties.direction).toBeDefined()
    const allows = iii.calls.filter(({ id }) => id === 'approval::add-always-allow')
    expect(allows.map(({ input }) => input.function_id)).toEqual(WRITER_READS)

    vi.setSystemTime(clockStart + 600_000)
    await writer.check('all')
    expect(iii.sent()[1].input.options).not.toHaveProperty('agent')
  })

  it('adopts a matching session that starts between ensure and send', async () => {
    const iii = new FakeIii()
    iii.raceAfterEnsure = true
    const writer = new TowerWriter(
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
    iii.status = { turn_id: 'old' }
    iii.meta.metadata.agent_profile = { id: 'another-agent' }
    const writer = new TowerWriter(
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
    iii.status = { turn_id: 'old' }
    iii.meta.metadata.agent_profile = 'tower-writer'
    const writer = new TowerWriter(
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
    iii.state.set('tower/summary:all', {
      result: result(),
      valid_hash: 'old',
      checked_at: new Date().toISOString(),
      failures: 1,
      last_attempt_at: Date.now(),
    })
    const writer = new TowerWriter(
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
    iii.state.set('tower/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: 'same',
      checked_at: '2026-11-22T10:00:00Z',
    })
    const changed = vi.fn()
    const writer = new TowerWriter(
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
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      changed,
    )
    await writer.check('all')

    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-1',
      status: 'failed',
      terminal: true,
      result_error: 'provider failed',
    })
    expect(writer.snapshot('all').error).toBe('update failed')
    expect(writer.snapshot('all').valid_hash).toBeNull()

    vi.setSystemTime(clockStart + 600_000)
    await writer.check('all')
    expect(writer.snapshot('all').error).toBe('update failed')
    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-2',
      status: 'completed',
      terminal: true,
      result: { nope: true },
    })
    expect(iii.sent()).toHaveLength(2)
    expect(writer.snapshot('all')).toMatchObject({ error: 'update failed', result: null })
  })

  it('boots the last valid summary after a failed turn and does not resend an unchanged hash', async () => {
    const iii = new FakeIii()
    iii.state.set('tower/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: 'still-current',
      checked_at: '2026-11-22T10:00:00Z',
    })
    const restarted = new TowerWriter(
      iii,
      () => config(),
      async () => data('still-current'),
      vi.fn(),
    )

    await restarted.check('all')

    expect(restarted.snapshot('all').result?.brief.headline).toBe('KAN-1 is moving.')
    expect(iii.sent()).toHaveLength(0)
  })

  it('accepts its own completion, persists it, and excludes no writer result event', async () => {
    const iii = new FakeIii()
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data('covered'),
      vi.fn(),
    )
    await writer.check('all')
    const summary = result()

    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-1',
      status: 'completed',
      terminal: true,
      result: summary,
    })

    expect(writer.snapshot('all')).toMatchObject({
      result: summary,
      valid_hash: 'covered',
      error: null,
    })
    expect(iii.state.get('tower/summary:all')).toMatchObject({ valid_hash: 'covered' })
  })

  it('ignores unknown and duplicate completion events', async () => {
    const iii = new FakeIii()
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data('covered'),
      vi.fn(),
    )
    await writer.check('all')
    const summary = result()

    await writer.onCompleted({
      session_id: 'tower-unrelated',
      turn_id: 'unknown',
      status: 'completed',
      terminal: true,
      result: summary,
    })
    expect(writer.snapshot('unrelated').result).toBeNull()

    const event = {
      session_id: 'tower-all',
      turn_id: 'turn-1',
      status: 'completed',
      terminal: true,
      result: summary,
    }
    await writer.onCompleted(event)
    await writer.onCompleted({ ...event, status: 'failed', result: undefined })
    expect(writer.snapshot('all')).toMatchObject({ valid_hash: 'covered', error: null })
  })

  it('does not let an older completion overwrite a newer covered hash', async () => {
    const iii = new FakeIii()
    let hash = 'old-hash'
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(hash),
      vi.fn(),
    )
    await writer.check('all')
    vi.setSystemTime(clockStart + 1)
    hash = 'new-hash'
    await writer.refresh('all')

    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-2',
      status: 'completed',
      terminal: true,
      result: result(),
    })
    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-1',
      status: 'completed',
      terminal: true,
      result: result(),
    })

    expect(writer.snapshot('all')).toMatchObject({ valid_hash: 'new-hash', error: null })
  })

  it('throttles a second refresh in the same interval with a stable refresh key', async () => {
    const iii = new FakeIii()
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await writer.refresh('all')
    const second = await writer.refresh('all')

    expect(iii.sent()).toHaveLength(1)
    expect(iii.sent()[0].input.idempotency_key).toMatch(/^tower:all:\d+:refresh$/)
    expect(second.throttled_until).toBe('2026-11-22T12:10:00.000Z')
  })

  it('keeps the refresh rate limit after a restart', async () => {
    const iii = new FakeIii()
    await new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    ).refresh('all')
    const restarted = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    expect((await restarted.refresh('all')).throttled_until).toBe('2026-11-22T12:10:00.000Z')
    expect(iii.sent()).toHaveLength(1)
  })

  it('turns setup and send RPC failures into update failed state', async () => {
    for (const functionId of [
      'session::ensure',
      'harness::status',
      'approval::set-mode',
      'harness::send',
    ]) {
      const iii = new FakeIii()
      iii.failOn = functionId
      const changed = vi.fn()
      const writer = new TowerWriter(
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
    const writer = new TowerWriter(
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
    iii.state.set('tower/summary:all', {
      result: result('2026-11-22T10:00:00Z'),
      valid_hash: 'old',
      checked_at: '2026-11-22T11:00:00Z',
    })
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data('new'),
      vi.fn(),
    )

    await writer.check('all')

    expect(JSON.parse(iii.sent()[0].input.message).checked_at).toBe('2026-11-22T11:00:00Z')
  })

  it('runs without approval-gate and reports a missing profile separately', async () => {
    const iii = new FakeIii()
    iii.approval = false
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    await writer.check('all')
    expect(iii.sent()).toHaveLength(1)
    expect(iii.calls.some(({ id }) => id.startsWith('approval::'))).toBe(false)

    const missing = new FakeIii()
    missing.profileMissing = true
    const withoutKit = new TowerWriter(
      missing,
      () => config(),
      async () => data(),
      vi.fn(),
    )
    expect(await withoutKit.check('all')).toMatchObject({
      error: 'writer profile missing: apply the tower kit',
      unavailable: true,
    })
    expect(missing.sent()).toHaveLength(0)
  })

  it('rechecks approval-gate after availability changes', async () => {
    const iii = new FakeIii()
    iii.approval = false
    const writer = new TowerWriter(
      iii,
      () => config(),
      async (scope) => data(scope),
      vi.fn(),
    )

    await writer.check('without-gate')
    expect(iii.calls.some(({ id }) => id.startsWith('approval::'))).toBe(false)

    iii.approval = true
    writer.invalidateAvailability()
    await writer.check('with-gate')
    expect(iii.calls.filter(({ id }) => id === 'approval::set-mode')).toHaveLength(1)
    expect(iii.calls.filter(({ id }) => id === 'approval::add-always-allow')).toHaveLength(
      WRITER_READS.length,
    )

    iii.approval = false
    writer.invalidateAvailability()
    await writer.check('removed-gate')
    expect(iii.calls.filter(({ id }) => id === 'approval::set-mode')).toHaveLength(1)
    expect(iii.sent()).toHaveLength(3)
  })

  it('does not cache a transient approval discovery failure as gate absence', async () => {
    const iii = new FakeIii()
    iii.failOn = 'engine::functions::list'
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    expect(await writer.check('all')).toMatchObject({ error: 'update failed' })
    expect(iii.sent()).toHaveLength(0)

    iii.failOn = null
    await writer.check('all', true)
    expect(iii.calls.filter(({ id }) => id === 'engine::functions::list')).toHaveLength(2)
    expect(iii.calls.filter(({ id }) => id === 'approval::set-mode')).toHaveLength(1)
    expect(iii.sent()).toHaveLength(1)
  })

  it('backs off after three failures and resets after a valid result', async () => {
    const iii = new FakeIii()
    let hash = 'h1'
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(hash),
      vi.fn(),
    )

    for (let index = 1; index <= 3; index += 1) {
      await writer.check('all')
      await writer.onCompleted({
        session_id: 'tower-all',
        turn_id: `turn-${index}`,
        status: 'failed',
        terminal: true,
      })
      vi.setSystemTime(clockStart + index * 600_000)
    }
    await writer.check('all')
    expect(iii.sent()).toHaveLength(3)

    vi.setSystemTime(clockStart + 4 * 600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(4)
    await writer.onCompleted({
      session_id: 'tower-all',
      turn_id: 'turn-4',
      status: 'completed',
      terminal: true,
      result: result(),
    })
    hash = 'h2'
    vi.setSystemTime(clockStart + 5 * 600_000)
    await writer.check('all')
    expect(iii.sent()).toHaveLength(5)
  })

  it('serializes concurrent first use per scope', async () => {
    const iii = new FakeIii()
    const writer = new TowerWriter(
      iii,
      () => config(),
      async () => data(),
      vi.fn(),
    )

    await Promise.all([writer.check('all'), writer.check('all')])

    expect(iii.sent()).toHaveLength(1)
  })
})
