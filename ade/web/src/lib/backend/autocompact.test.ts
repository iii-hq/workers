import { describe, expect, it, vi } from 'vitest'
import type { IiiClient } from '@/lib/iii-client'
import {
  AUTOCOMPACT_LOG_FUNCTION_ID,
  AUTOCOMPACT_MODELS_FUNCTION_ID,
  listAutocompactLog,
  listAutocompactModels,
  parseAutocompactLog,
  parseAutocompactModels,
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
