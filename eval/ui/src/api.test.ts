import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createEvalApi } from './api'
import type { CriterionInput, MonitorConfig } from './types'

function fakeApi() {
  const trigger = vi.fn().mockResolvedValue({})
  return { trigger, api: createEvalApi({ iii: { trigger } } as unknown as Host) }
}

const CONFIG: MonitorConfig = {
  enabled: true,
  model: { model: 'claude-sonnet-5', provider: 'anthropic' },
  code_repository: '/home/layon/workspaces/workers',
  revision: 'r1',
  updated_at: 1,
}

describe('eval::configure', () => {
  it('names the directory only when there is one: absent turns code access off', async () => {
    const { api, trigger } = fakeApi()
    await api.configure(true, CONFIG.model)
    await api.configure(true, CONFIG.model, '/srv/workers')
    expect(trigger.mock.calls[0][1]).toEqual({ enabled: true, model: CONFIG.model })
    expect(trigger.mock.calls[0][1]).not.toHaveProperty('code_repository')
    expect(trigger.mock.calls[1][1]).toEqual({ enabled: true, model: CONFIG.model, code_repository: '/srv/workers' })
  })

  it('keeps the saved directory when observation is paused and resumed', async () => {
    const { api, trigger } = fakeApi()
    await api.toggle(CONFIG)
    await api.toggle({ ...CONFIG, enabled: false })
    expect(trigger.mock.calls[0][0]).toBe('eval::configure')
    expect(trigger.mock.calls[0][1]).toEqual({
      enabled: false,
      model: CONFIG.model,
      code_repository: '/home/layon/workspaces/workers',
    })
    expect(trigger.mock.calls[1][1]).toMatchObject({
      enabled: true,
      code_repository: '/home/layon/workspaces/workers',
    })
    // A configuration without a directory stays without one.
    const { code_repository: _gone, ...plain } = CONFIG
    await api.toggle(plain)
    expect(trigger.mock.calls[2][1]).not.toHaveProperty('code_repository')
  })

  it('names the daily cap only when there is one, and keeps it when observation is paused and resumed', async () => {
    const { api, trigger } = fakeApi()
    await api.configure(true, CONFIG.model)
    await api.configure(true, CONFIG.model, undefined, 2.5)
    await api.toggle({ ...CONFIG, daily_cost_cap_usd: 4 })
    await api.toggle(CONFIG)
    expect(trigger.mock.calls[0][1]).not.toHaveProperty('daily_cost_cap_usd')
    expect(trigger.mock.calls[1][1]).toMatchObject({ daily_cost_cap_usd: 2.5 })
    expect(trigger.mock.calls[2][1]).toMatchObject({ enabled: false, daily_cost_cap_usd: 4 })
    expect(trigger.mock.calls[3][1]).not.toHaveProperty('daily_cost_cap_usd')
  })
})

describe('eval::list', () => {
  it('filters by observed turn only when asked', async () => {
    const { api, trigger } = fakeApi()
    trigger.mockResolvedValue({ evaluations: [] })
    await api.list()
    await api.list('sess_1:turn_1')
    expect(trigger.mock.calls[0]).toEqual(['eval::list', { limit: 200 }, { timeoutMs: 30_000 }])
    expect(trigger.mock.calls[1][1]).toEqual({ limit: 200, observation_key: 'sess_1:turn_1' })
  })
})

const CRITERION: CriterionInput = {
  metric: 'signal_per_run',
  pattern: 'repeated_contract_discovery:crm::profile',
  direction: 'decrease',
  min_effect: 0.5,
  min_runs: 3,
}

describe('review and validation', () => {
  it('names no author unless a change does: the monitor credits the user it runs as', async () => {
    const { api, trigger } = fakeApi()
    await api.review('eval_1', 2, { action: 'set_lifecycle', status: 'shipped', pr: 'iii-hq/workers#1300' })
    await api.review('eval_1', 2, { action: 'set_criterion', criterion: CRITERION, by: 'ana' })
    expect(trigger.mock.calls[0][0]).toBe('eval::review')
    expect(trigger.mock.calls[0][1]).toEqual({
      evaluation_id: 'eval_1',
      suggestion_index: 2,
      action: 'set_lifecycle',
      status: 'shipped',
      pr: 'iii-hq/workers#1300',
    })
    expect(trigger.mock.calls[1][1]).toMatchObject({ action: 'set_criterion', criterion: CRITERION, by: 'ana' })
  })

  it('lists the reviews of one analysis or of all', async () => {
    const { api, trigger } = fakeApi()
    await api.reviews()
    await api.reviews('eval_1')
    expect(trigger.mock.calls[0].slice(0, 2)).toEqual(['eval::reviews', {}])
    expect(trigger.mock.calls[1][1]).toEqual({ evaluation_id: 'eval_1' })
  })

  it('starts a validation with the long timeout', async () => {
    const { api, trigger } = fakeApi()
    await api.startValidation('eval_1', 0, {
      scenario_id: 'alertmanager_route_match',
      candidate_ref: 'feat/fix',
      runs: 5,
      model: 'deepseek-flash',
      provider: 'deepseek',
      criterion: CRITERION,
    })
    expect(trigger.mock.calls[0][0]).toBe('eval::start-validation')
    expect(trigger.mock.calls[0][1]).toEqual({
      evaluation_id: 'eval_1',
      suggestion_index: 0,
      scenario_id: 'alertmanager_route_match',
      candidate_ref: 'feat/fix',
      runs: 5,
      model: 'deepseek-flash',
      provider: 'deepseek',
      criterion: CRITERION,
    })
    expect(trigger.mock.calls[0][2]).toEqual({ timeoutMs: 90_000 })
  })

  it('resolves the refs of a validation with a dry run, never a start', async () => {
    const { api, trigger } = fakeApi()
    await api.resolveValidation('eval_1', 0, {
      scenario_id: 'alertmanager_route_match',
      candidate_ref: 'feat/fix',
      baseline_ref: 'main',
      runs: 5,
    })
    expect(trigger.mock.calls[0][0]).toBe('eval::start-validation')
    expect(trigger.mock.calls[0][1]).toEqual({
      evaluation_id: 'eval_1',
      suggestion_index: 0,
      scenario_id: 'alertmanager_route_match',
      candidate_ref: 'feat/fix',
      baseline_ref: 'main',
      runs: 5,
      dry_run: true,
    })
    expect(trigger.mock.calls[0][2]).toEqual({ timeoutMs: 30_000 })
  })

  it('asks the recurrence of a suggestion', async () => {
    const { api, trigger } = fakeApi()
    await api.recurrence('eval_1', 3)
    expect(trigger.mock.calls[0].slice(0, 2)).toEqual([
      'eval::recurrence',
      { evaluation_id: 'eval_1', suggestion_index: 3 },
    ])
  })
})

describe('e2e scenarios', () => {
  it('reads every page of the catalog and keeps the id, title and summary', async () => {
    const { api, trigger } = fakeApi()
    trigger
      .mockResolvedValueOnce({
        rows: [{ test_id: 'a', spec: { title: 'A', summary: 'first' } }],
        next_cursor: 'sha256:x:1',
      })
      .mockResolvedValueOnce({ rows: [{ test_id: 'b' }], next_cursor: null })
    expect(await api.e2eScenarios()).toEqual([
      { id: 'a', title: 'A', summary: 'first' },
      { id: 'b', title: '', summary: '' },
    ])
    expect(trigger.mock.calls[0][1]).toEqual({ limit: 100 })
    expect(trigger.mock.calls[1][1]).toEqual({ limit: 100, cursor: 'sha256:x:1' })
  })

  it('stops following a cursor that never ends', async () => {
    const { api, trigger } = fakeApi()
    trigger.mockResolvedValue({ rows: [], next_cursor: 'again' })
    await api.e2eScenarios()
    expect(trigger).toHaveBeenCalledTimes(10)
  })
})

describe('e2e::dashboard::executions-list', () => {
  it('reads the newest 100, or only the executions named', async () => {
    const { api, trigger } = fakeApi()
    await api.e2eExecutions()
    await api.e2eExecutions(['plan-1', 'plan-2'])
    expect(trigger.mock.calls.map(([id]) => id)).toEqual(Array(2).fill('e2e::dashboard::executions-list'))
    expect(trigger.mock.calls[0][1]).toEqual({ limit: 100 })
    expect(trigger.mock.calls[1][1]).toEqual({ ids: ['plan-1', 'plan-2'] })
  })
})
