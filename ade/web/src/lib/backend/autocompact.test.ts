import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import {
  AUTOCOMPACT_LOG_FUNCTION_ID,
  AUTOCOMPACT_MODELS_FUNCTION_ID,
  listAutocompactLog,
  listAutocompactModels,
  parseAutocompactLog,
  parseAutocompactModels,
  startTurnAutocompactSubscription,
} from './autocompact'

vi.mock('@/lib/iii-client', () => ({ getIiiClient: vi.fn() }))

function fakeClient(answer: unknown) {
  const call = vi.fn().mockResolvedValue(answer)
  return { client: { call } as unknown as IiiClient, call }
}

describe('harness::autocompact::models', () => {
  it('reads the models, the shortest budget first', async () => {
    const { client, call } = fakeClient([
      {
        provider: 'anthropic',
        model: 'claude-sonnet-5-5',
        name: 'Sonnet 5.5',
        context_window: 1_000_000,
        usable: 916_000,
        last_used_ms: 1_700_000_000_000,
      },
      {
        provider: 'anthropic',
        model: 'claude-opus-5-5',
        name: 'Opus 5.5',
        context_window: 200_000,
        usable: 116_000,
        last_used_ms: 1_700_000_100_000,
      },
    ])
    const models = await listAutocompactModels(client)
    expect(call).toHaveBeenCalledWith(AUTOCOMPACT_MODELS_FUNCTION_ID, {})
    expect(models.map((m) => [m.name, m.usable])).toEqual([
      ['Opus 5.5', 116_000],
      ['Sonnet 5.5', 916_000],
    ])
  })

  it('takes a wrapped list, names a model by its id, and drops broken rows', () => {
    expect(
      parseAutocompactModels({
        models: [
          { provider: 'x', model: 'local-7b', usable: 30_000 },
          { model: 'no-usable' },
          { usable: 50_000 },
          'junk',
        ],
      }),
    ).toEqual([
      {
        provider: 'x',
        model: 'local-7b',
        name: 'local-7b',
        context_window: null,
        usable: 30_000,
        last_used_ms: 0,
      },
    ])
    expect(parseAutocompactModels(null)).toEqual([])
  })
})

describe('harness::autocompact::log', () => {
  it('asks for the last 20 and reads them newest first', async () => {
    const { client, call } = fakeClient([
      {
        session_id: 's-old',
        title: 'Ledger reconciliation',
        at_ms: 1_000,
        trigger: 'auto',
        threshold_tokens: 160_000,
        context_before: 161_000,
        context_after: 34_000,
        rule: { scope: 'project', project_id: 'p', project_name: 'acme-web' },
      },
      {
        session_id: 's-new',
        title: 'Fix the flaky test',
        at_ms: 2_000,
        trigger: 'inline',
        threshold_tokens: 400_000,
        context_before: 412_000,
        context_after: 51_000,
        rule: { scope: 'global' },
      },
    ])
    const log = await listAutocompactLog(undefined, client)
    expect(call).toHaveBeenCalledWith(AUTOCOMPACT_LOG_FUNCTION_ID, {
      limit: 20,
    })
    expect(log.map((e) => e.session_id)).toEqual(['s-new', 's-old'])
    expect(log[1]).toEqual({
      session_id: 's-old',
      title: 'Ledger reconciliation',
      at_ms: 1_000,
      trigger: 'auto',
      threshold_tokens: 160_000,
      context_before: 161_000,
      context_after: 34_000,
      rule: { scope: 'project', project_id: 'p', project_name: 'acme-web' },
    })
  })

  it('tolerates a wrapped list, a missing title and an unknown trigger', () => {
    expect(
      parseAutocompactLog({
        entries: [{ session_id: 's1', trigger: 'surprise' }, { title: 'x' }],
      }),
    ).toEqual([
      { session_id: 's1', title: 's1', at_ms: 0, trigger: 'auto', rule: null },
    ])
    expect(parseAutocompactLog({ log: [] })).toEqual([])
    expect(parseAutocompactLog(undefined)).toEqual([])
  })
})

describe("a running turn's snapshot", () => {
  function fakeObserver() {
    const handlers = new Map<string, (payload: unknown) => void>()
    const offHandler = vi.fn()
    const offObserver = vi.fn()
    const client = {
      browserId: 'b1',
      on: vi.fn((fn: string, handler: (payload: unknown) => void) => {
        handlers.set(fn, handler)
        return offHandler
      }),
      observe: vi.fn(() => offObserver),
    }
    const fire = (payload: unknown) =>
      handlers.get('iii::console::context_snapshot')?.(payload)
    return { client, fire, offHandler, offObserver }
  }

  const SNAPSHOT = {
    session_id: 's1',
    total: 142_000,
    usable: 916_000,
    autocompact: {
      setting_tokens: 160_000,
      effective_tokens: 160_000,
      cap_tokens: 180_000,
      rule: { scope: 'project', project_id: 'p1', project_name: 'acme-web' },
    },
  }

  it("observes the session's harness_context key, internally", () => {
    const { client } = fakeObserver()
    startTurnAutocompactSubscription(client as never, 's1', vi.fn())
    expect(client.observe).toHaveBeenCalledWith(
      {
        function_id: 'state::on-changed',
        filters: {
          'input.scope': { $eq: 'harness_context' },
          'input.key': { $eq: 's1' },
        },
      },
      'iii::console::context_snapshot::b1',
    )
  })

  it("hands over each step's auto-compact", () => {
    const { client, fire } = fakeObserver()
    const onValue = vi.fn()
    startTurnAutocompactSubscription(client as never, 's1', onValue)
    fire({ key: 's1', event_type: 'state:updated', new_value: SNAPSHOT })
    expect(onValue).toHaveBeenCalledWith({
      setting_tokens: 160_000,
      effective_tokens: 160_000,
      cap_tokens: 180_000,
      rule: { scope: 'project', project_id: 'p1', project_name: 'acme-web' },
    })
  })

  it("skips another session's write, a delete and an older harness's snapshot", () => {
    const { client, fire } = fakeObserver()
    const onValue = vi.fn()
    startTurnAutocompactSubscription(client as never, 's1', onValue)
    // The handler id is shared: another chat's observer delivers here too.
    fire({ key: 's2', event_type: 'state:updated', new_value: SNAPSHOT })
    fire({
      key: 's1',
      event_type: 'state:updated',
      new_value: { ...SNAPSHOT, session_id: 's2' },
    })
    fire({ key: 's1', event_type: 'state:deleted', new_value: null })
    fire({
      key: 's1',
      event_type: 'state:updated',
      new_value: { ...SNAPSHOT, autocompact: undefined },
    })
    fire(null)
    expect(onValue).not.toHaveBeenCalled()
  })

  it('closes the handler and the observer', () => {
    const { client, offHandler, offObserver } = fakeObserver()
    const off = startTurnAutocompactSubscription(client as never, 's1', vi.fn())
    off()
    expect(offHandler).toHaveBeenCalledOnce()
    expect(offObserver).toHaveBeenCalledOnce()
  })
})
