import { describe, expect, it } from 'vitest'
import { e2eRuns, runDescription, runGroups, sameCase, withPicked } from './e2e-runs'

const NOW = Date.parse('2026-10-02T16:00:00Z')

const response = {
  executions: [
    {
      id: 'plan-old',
      label: 'A/B · BASE cc6b778',
      status: 'passed',
      started_at: '2026-09-30T16:35:02Z',
      parameters: {
        model: 'deepseek-flash',
        provider: 'deepseek',
        scenarios: ['tool_contract_recovery', 'timer_wake'],
      },
      stack: [{ name: 'harness', observed: '1.8.39', commit: 'cc6b778aa', dirty: false }],
    },
    {
      id: 'plan-new',
      label: '',
      status: 'technical_failed',
      started_at: '2026-10-02T14:00:00Z',
      parameters: { model: 'deepseek-flash', provider: 'deepseek', scenarios: ['kanban_c1_foundation'] },
      stack: [{ name: 'harness', observed: '1.8.42', commit: 'f3a49e1bb', dirty: true }],
    },
    {
      // An older entry: no parameters, model and scenarios from subjects / metrics.
      id: '5088a537',
      label: 'Live check',
      status: 'passed',
      started_at: 'not a date',
      subjects: [{ model: 'deepseek-flash', provider: 'deepseek', scenarios: [{ id: 'tool_contract_recovery' }] }],
      scenario_metrics: [{ scenario_id: 'tool_contract_recovery' }],
      stack: { mode: 'source' },
    },
    {
      id: 'plan-cand',
      label: 'A/B · CANDIDATA ab17d27',
      status: 'passed',
      started_at: '2026-09-30T16:35:03Z',
      parameters: {
        model: 'deepseek-flash',
        provider: 'deepseek',
        scenarios: ['timer_wake', 'tool_contract_recovery'],
      },
    },
    { label: 'no id' },
  ],
}

describe('e2e runs', () => {
  const runs = e2eRuns(response)
  const byId = (id: string) => runs.find((run) => run.id === id)!

  it('keeps what the picker shows and skips runs without an id', () => {
    expect(runs.map((run) => run.id)).toEqual(['plan-old', 'plan-new', '5088a537', 'plan-cand'])
    expect(byId('plan-old')).toMatchObject({
      harness: '1.8.39 · cc6b778',
      scenarios: ['timer_wake', 'tool_contract_recovery'],
    })
    expect(byId('plan-new')).toMatchObject({ label: 'Unnamed run', harness: '1.8.42 · f3a49e1 · uncommitted changes' })
    expect(byId('5088a537')).toMatchObject({
      model: 'deepseek-flash',
      provider: 'deepseek',
      scenarios: ['tool_contract_recovery'],
      harness: undefined,
      startedAt: undefined,
    })
    expect(e2eRuns(null)).toEqual([])
  })

  it('treats the same model, provider and scenario set as the same case', () => {
    expect(sameCase(byId('plan-old'), byId('plan-cand'))).toBe(true)
    expect(sameCase(byId('plan-old'), byId('5088a537'))).toBe(false)
  })

  it('names the plan scenario first in the description', () => {
    expect(runDescription(byId('plan-old'), 'tool_contract_recovery', NOW)).toMatch(
      /^passed · Harness 1\.8\.39 · cc6b778 · deepseek-flash · tool_contract_recovery \+1 · /,
    )
    expect(runDescription(byId('plan-new'), null, NOW)).toMatch(/^technical failed · .* · kanban_c1_foundation · /)
  })

  it('says the age in words, rounded down', () => {
    expect(runDescription(byId('plan-new'), null, NOW)).toMatch(/ · 2 hours ago$/)
    expect(runDescription(byId('plan-old'), null, NOW)).toMatch(/ · 1 day ago$/)
    const at = (minutes: number) => ({ ...byId('plan-new'), startedAt: NOW - minutes * 60_000 })
    expect(runDescription(at(12), null, NOW)).toMatch(/ · 12 minutes ago$/)
    expect(runDescription(at(0.5), null, NOW)).toMatch(/ · just now$/)
    expect(runDescription(at(60 * 24 * 40), null, NOW)).toMatch(/ · 1 month ago$/)
  })

  it('searches a long id by its readable prefix only', () => {
    const [group] = runGroups(
      [{ ...byId('plan-old'), id: 'plan-4074486e3e3942713bae30ec6a0c6b34' }],
      null,
      undefined,
      'baseline',
      NOW,
    )
    expect(group.options[0].keywords?.[0]).toBe('plan-4074486e')
  })

  it('groups runs with the plan scenario first and blocks the other side', () => {
    const groups = runGroups(runs, 'tool_contract_recovery', byId('plan-old'), 'baseline', NOW)
    expect(groups.map((group) => group.label)).toEqual([
      'Same scenarios and model as the baseline',
      'Other runs with tool_contract_recovery',
      'Other runs',
    ])
    expect(groups[0].options.map((option) => [option.value, option.disabled])).toEqual([
      ['plan-cand', false],
      ['plan-old', true],
    ])
    expect(groups[1].options.map((option) => option.value)).toEqual(['5088a537'])
    expect(groups[2].options[0].label).toBe('Unnamed run · plan-new')
  })

  it('without another run picked, or with one that is not a run of the plan scenario, lists the scenario first', () => {
    expect(runGroups(runs, 'tool_contract_recovery', undefined, 'baseline', NOW).map((group) => group.label)).toEqual([
      'Runs with tool_contract_recovery',
      'Other runs',
    ])
    expect(
      runGroups(runs, 'tool_contract_recovery', byId('plan-new'), 'baseline', NOW).map((group) => group.label),
    ).toEqual(['Runs with tool_contract_recovery', 'Other runs'])
  })

  it('says why the run on the other side is disabled and makes every run searchable by what tells it apart', () => {
    const groups = runGroups(runs, 'tool_contract_recovery', byId('plan-old'), 'baseline', NOW)
    const taken = groups[0].options.find((option) => option.value === 'plan-old')!
    expect(taken.description).toBe('Already chosen as the baseline · Harness 1.8.39 · cc6b778')
    expect(taken.keywords).toEqual([
      'plan-old',
      '1.8.39 · cc6b778',
      'deepseek-flash',
      'timer_wake',
      'tool_contract_recovery',
    ])
    const bare = runGroups(runs, null, byId('5088a537'), 'candidate', NOW)[0].options.find(
      (option) => option.value === '5088a537',
    )!
    expect(bare.description).toBe('Already chosen as the candidate')
  })

  it('lists runs that did not finish after the ones that did, newest first in each', () => {
    const more = e2eRuns({
      executions: [
        { id: 'running', status: 'running', started_at: '2026-10-02T15:50:00Z', parameters: { scenarios: ['s'] } },
        { id: 'old', status: 'failed', started_at: '2026-09-01T10:00:00Z', parameters: { scenarios: ['s'] } },
        {
          id: 'tech',
          status: 'technical_failed',
          started_at: '2026-10-02T12:00:00Z',
          parameters: { scenarios: ['s'] },
        },
        { id: 'new', status: 'passed', started_at: '2026-10-02T14:00:00Z', parameters: { scenarios: ['s'] } },
      ],
    })
    const [group] = runGroups(more, 's', undefined, 'baseline', NOW)
    expect(group.options.map((option) => option.value)).toEqual(['new', 'old', 'running', 'tech'])
  })

  it('without a plan scenario, puts runs of the same case as the other side first', () => {
    expect(runGroups(runs, null, undefined, 'baseline', NOW).map((group) => group.label)).toEqual(['Recent runs'])
    const groups = runGroups(runs, null, byId('plan-old'), 'baseline', NOW)
    expect(groups[0]).toMatchObject({ label: 'Same scenarios and model as the baseline' })
    expect(groups[0].options.map((option) => option.value)).toEqual(['plan-cand', 'plan-old'])
  })

  it('shows a picked id the list does not hold', () => {
    const groups = runGroups(runs, null, undefined, 'baseline', NOW)
    expect(withPicked(groups, 'plan-old')).toBe(groups)
    expect(withPicked(groups, '')).toBe(groups)
    expect(withPicked(groups, 'exe_new')[1]).toEqual({ label: '', options: [{ value: 'exe_new', label: 'exe_new' }] })
  })
})
