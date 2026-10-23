import { describe, expect, it } from 'vitest'
import type { ValidationResolution } from '../../../types'
import {
  checkFor,
  classifyStartFailure,
  costApprox,
  defaultModel,
  dockerHistory,
  estimateFor,
  executionProgress,
  minutesApprox,
  needsCheck,
  parseRuns,
  type RefCheck,
  refInputs,
  refKey,
  refusedAnswer,
  resolvedAnswer,
  resolvedLine,
  sentence,
  settleCheck,
  startAllowed,
} from './start-validation-model'

const past = (over: Record<string, unknown> = {}) => ({
  status: 'passed',
  started_at: '2026-10-02T15:00:00Z',
  completed_at: '2026-10-02T15:10:00Z',
  parameters: {
    model: 'deepseek-flash',
    provider: 'deepseek',
    runs: 5,
    scenarios: ['tool_contract_recovery'],
    where: 'docker',
  },
  scenario_metrics: [
    { scenario_id: 'tool_contract_recovery', averages: { cost_usd: 0.004 }, samples: { cost_usd: 5 } },
  ],
  ...over,
})

const query = { scenarioId: 'tool_contract_recovery', model: 'deepseek-flash', provider: 'deepseek', runs: 5 }

describe('estimate', () => {
  it('scales the median past cost and time per run to both sides and the runs asked for', () => {
    const estimate = estimateFor(
      {
        executions: [
          past(),
          past({ completed_at: '2026-10-02T15:20:00Z' }),
          // An outlier does not move a median: 56 minutes a run against 2 and 4.
          past({ completed_at: '2026-10-02T19:40:00Z' }),
        ],
      },
      query,
    )
    expect(estimate?.executions).toBe(3)
    // 0.004 USD a run × 5 runs × 2 sides.
    expect(estimate?.cost?.usd).toBeCloseTo(0.04)
    expect(estimate?.cost?.from).toBe(3)
    // 10, 20 and 280 min for 5 runs → 2, 4 and 56 min a run: the median 4 → 20 min for 5.
    expect(estimate?.minutes).toEqual({ value: 20, from: 3 })
  })

  it('shows no figure from fewer than three past executions', () => {
    expect(estimateFor({ executions: [past(), past()] }, query)).toBeUndefined()
    expect(estimateFor({ executions: [past(), past(), past()] }, query)?.executions).toBe(3)
  })

  it('counts only Docker executions of the same scenario and model that finished their runs', () => {
    const list = {
      executions: [
        past({ parameters: { model: 'other', provider: 'deepseek', runs: 5, scenarios: ['x'], where: 'docker' } }),
        past({ parameters: { model: 'deepseek-flash', provider: 'deepseek', runs: 5, where: 'github' } }),
        past({ parameters: { model: 'deepseek-flash', provider: 'deepseek', runs: 5 } }),
        past({ status: 'cancelled' }),
        past({ status: 'running' }),
        past({ scenario_metrics: [{ scenario_id: 'other', averages: { cost_usd: 1 }, samples: { cost_usd: 5 } }] }),
      ],
    }
    expect(estimateFor(list, query)).toBeUndefined()
    expect(estimateFor({ executions: [...list.executions, past(), past(), past()] }, query)?.executions).toBe(3)
    expect(estimateFor({}, query)).toBeUndefined()
  })

  it('leaves out a cost nobody reported instead of calling it zero, and says how many reported one', () => {
    const noCost = past({
      scenario_metrics: [
        { scenario_id: 'tool_contract_recovery', averages: { cost_usd: 0 }, samples: { cost_usd: 0 } },
      ],
    })
    const none = estimateFor({ executions: [noCost, noCost, noCost] }, query)
    expect(none?.executions).toBe(3)
    expect(none?.cost).toBeUndefined()
    expect(none?.minutes).toBeDefined()
    // One of three reported it: the figure says so instead of passing for three.
    const one = estimateFor({ executions: [past(), noCost, noCost] }, query)
    expect(one?.cost?.from).toBe(1)
  })

  it('words the figures', () => {
    expect(costApprox(0.45)).toBe('≈ $0.45')
    expect(costApprox(0.0039)).toBe('≈ $0.0039')
    expect(minutesApprox(11.6)).toBe('about 12 min')
    expect(minutesApprox(0.2)).toBe('about 1 min')
  })
})

describe('the model Docker can run', () => {
  const docker = (model: string, provider: string, started_at: string, status = 'passed') =>
    past({ status, started_at, parameters: { model, provider, runs: 1, where: 'docker' } })
  const history = dockerHistory({
    executions: [
      docker('deepseek-flash', 'deepseek', '2026-09-30T16:35:03Z'),
      docker('claude-opus-5-5', 'anthropic', '2026-09-29T14:51:32Z'),
      // Failed or cancelled executions prove nothing about the provider; other places do not either.
      docker('glm-5', 'zai', '2026-10-01T10:00:00Z', 'failed'),
      past({ parameters: { model: 'gpt-5', provider: 'openai', runs: 1, where: 'github' } }),
    ],
  })

  it('knows the providers its passed Docker executions used, and the newest model', () => {
    expect([...history.providers].sort()).toEqual(['anthropic', 'deepseek'])
    expect(history.latest).toEqual({ model: 'deepseek-flash', provider: 'deepseek' })
    expect(dockerHistory({}).providers.size).toBe(0)
    expect(dockerHistory(null).latest).toBeUndefined()
  })

  it('keeps the observed model when Docker ran its provider, else takes the last one Docker ran', () => {
    const anthropic = { model: 'claude-sonnet-5', provider: 'anthropic' }
    expect(defaultModel(anthropic, history)).toEqual(anthropic)
    // A CLI provider the stack does not carry would fail after the Docker build.
    expect(defaultModel({ model: 'claude-opus-5-5', provider: 'claude-code' }, history)).toEqual({
      model: 'deepseek-flash',
      provider: 'deepseek',
    })
    // Nothing known about Docker, or nothing observed: no reason to change the choice.
    const code = { model: 'claude-opus-5-5', provider: 'claude-code' }
    expect(defaultModel(code, undefined)).toEqual(code)
    expect(defaultModel(code, dockerHistory({}))).toEqual(code)
    expect(defaultModel(undefined, history)).toEqual({ model: 'deepseek-flash', provider: 'deepseek' })
    expect(defaultModel(undefined, undefined)).toBeUndefined()
  })
})

describe('runs', () => {
  it('takes 1 to 20, at least what the criterion needs', () => {
    expect(parseRuns('5', 5)).toEqual({ runs: 5 })
    expect(parseRuns('0', 3)).toHaveProperty('error')
    expect(parseRuns('21', 3)).toHaveProperty('error')
    expect(parseRuns('2.5', 3)).toHaveProperty('error')
    expect(parseRuns('3', 5)).toEqual({ error: 'The criterion needs at least 5 runs per side.' })
  })
})

describe('refusals', () => {
  it('puts a git refusal under the side it names', () => {
    expect(classifyStartFailure('git_ref_invalid: the candidate ref `x` is not a commit of /w')).toMatchObject({
      field: 'candidate',
    })
    expect(classifyStartFailure('git_ref_invalid: the baseline ref `y` is not a commit of /w').field).toBe('baseline')
    expect(
      classifyStartFailure(
        'git_ref_invalid: baseline and candidate are the same commit 8c25e07; a candidate already in',
      ).field,
    ).toBe('baseline')
    expect(
      classifyStartFailure('git_ref_invalid: /w has no merge base of the candidate and origin/main; pass baseline_ref')
        .field,
    ).toBe('baseline')
    expect(classifyStartFailure('commit_not_pushed: the candidate commit abc is on no remote branch').field).toBe(
      'candidate',
    )
    expect(classifyStartFailure('commit_not_pushed: the baseline commit abc is on no remote branch').field).toBe(
      'baseline',
    )
  })

  it('tells a busy E2E from one that does not answer, and strips the prefix', () => {
    expect(
      classifyStartFailure('eval::start-validation failed: e2e_busy: the E2E did not start the execution: busy'),
    ).toEqual({
      field: 'e2e',
      busy: true,
      text: 'the E2E did not start the execution: busy',
    })
    expect(classifyStartFailure('e2e_unavailable: the E2E service could not list its stacks')).toEqual({
      field: 'e2e',
      text: 'the E2E service could not list its stacks',
    })
  })

  it('tells an E2E that never answered, where the execution may exist, from a refusal', () => {
    expect(
      classifyStartFailure(
        'eval::start-validation failed: dependency error: e2e_start_unconfirmed: the E2E did not answer, so the execution may have started anyway: look for `eval e S1 candidate` in the E2E before starting again (dependency error: e2e::dashboard::execution-start exceeded its 30000 ms timeout)',
      ),
    ).toMatchObject({ field: 'e2e', unconfirmed: true })
    expect(classifyStartFailure('e2e_busy: busy').unconfirmed).toBeUndefined()
  })

  it('knows the other refusals', () => {
    expect(classifyStartFailure('criterion_frozen: results already exist').field).toBe('criterion')
    expect(
      classifyStartFailure('the criterion needs 5 completed runs on each side but only 3 are requested').field,
    ).toBe('runs')
    expect(classifyStartFailure('code_repository_required: configure the codebase directory').field).toBe('other')
    expect(classifyStartFailure('something unexpected')).toEqual({ field: 'other', text: 'something unexpected' })
  })
})

describe('progress', () => {
  const list = {
    executions: [
      { id: 'plan-a', plan_execution: { state: 'running', finished: 3, completed: 2, planned: 5 } },
      { id: 'plan-b', plan_execution: { state: 'completed', completed: 5, planned: 5 } },
      { id: 'plan-c' },
    ],
  }

  it('reads the finished and planned runs of an execution', () => {
    expect(executionProgress(list, 'plan-a')).toEqual({ state: 'running', finished: 3, planned: 5 })
    expect(executionProgress(list, 'plan-b')).toEqual({ state: 'completed', finished: 5, planned: 5 })
  })

  it('knows nothing it was not told', () => {
    expect(executionProgress(list, 'plan-c')).toBeUndefined()
    expect(executionProgress(list, 'plan-z')).toBeUndefined()
    expect(executionProgress(list, undefined)).toBeUndefined()
    expect(executionProgress(null, 'plan-a')).toBeUndefined()
  })
})

describe('sentence', () => {
  it("capitalises the backend's clause and closes it", () => {
    expect(sentence('the candidate ref `x` is not a commit')).toBe('The candidate ref `x` is not a commit.')
    expect(sentence('Already done.')).toBe('Already done.')
    expect(sentence('  busy?  ')).toBe('Busy?')
    expect(sentence('')).toBe('')
  })
})

describe('the refs check', () => {
  const form: Parameters<typeof refInputs>[0] = {
    scenario: 'tool_contract_recovery',
    candidate: ' feat/fix ',
    baseline: null,
    runs: 5,
  }
  const resolution: ValidationResolution = {
    baseline: { commit: 'a'.repeat(40), short: 'a'.repeat(12), branch: 'origin/main' },
    candidate: { commit: 'b'.repeat(40), short: 'b'.repeat(12), branch: 'origin/feat/fix' },
    warnings: [],
  }
  const asked = (over: Partial<typeof form> = {}) => {
    const inputs = refInputs({ ...form, ...over })
    if (!inputs) throw new Error('inputs expected')
    return refKey(inputs)
  }

  it('asks only once the form has what a dry run needs, and trims what was typed', () => {
    expect(refInputs(form)).toEqual({
      scenario: 'tool_contract_recovery',
      candidate: 'feat/fix',
      baseline: null,
      runs: 5,
    })
    expect(refInputs({ ...form, candidate: '  ' })).toBeUndefined()
    expect(refInputs({ ...form, scenario: '' })).toBeUndefined()
    expect(refInputs({ ...form, runs: undefined })).toBeUndefined()
    // A baseline the person chose to write but has not written is not the default either.
    expect(refInputs({ ...form, baseline: ' ' })).toBeUndefined()
    expect(refInputs({ ...form, baseline: ' main ' })?.baseline).toBe('main')
  })

  it('keys the answer to every input it depends on, and to nothing else', () => {
    expect(asked({ candidate: 'feat/fix' })).toBe(asked())
    for (const change of [{ candidate: 'other' }, { baseline: 'main' }, { scenario: 's2' }, { runs: 6 }]) {
      expect(asked(change)).not.toBe(asked())
    }
    // The merge-base default and a typed baseline are different questions.
    expect(asked({ baseline: 'origin/main' })).not.toBe(asked({ baseline: null }))
  })

  it('says what was resolved and where it is pushed', () => {
    expect(resolvedLine(resolution.candidate)).toBe('Resolved · bbbbbbbbbbbb · pushed to origin/feat/fix')
  })

  it('puts each refusal under the field it names, without the machine prefix', () => {
    const refused = (message: string) => {
      const answer = refusedAnswer('k', message)
      if (answer.phase !== 'refused') throw new Error('refusal expected')
      return [answer.field, answer.text]
    }
    expect(refused('git_ref_invalid: the candidate ref `nope` is not a commit of /w')).toEqual([
      'candidate',
      'the candidate ref `nope` is not a commit of /w',
    ])
    expect(
      refused('eval::start-validation failed: commit_not_pushed: the candidate commit abc is on no remote branch'),
    ).toEqual(['candidate', 'the candidate commit abc is on no remote branch'])
    expect(refused('commit_not_pushed: the baseline commit abc is on no remote branch')[0]).toBe('baseline')
    expect(refused('git_ref_invalid: the baseline ref `y` is not a commit of /w')[0]).toBe('baseline')
    expect(refused('git_ref_invalid: baseline and candidate are the same commit abc; a candidate already in')[0]).toBe(
      'baseline',
    )
    expect(
      refused('git_ref_invalid: /w has no merge base of the candidate and origin/main; pass baseline_ref')[0],
    ).toBe('baseline')
    // What no ref owns is a general line, not an error of what was typed.
    expect(refused('git_unavailable: could not run git')[0]).toBe('other')
    expect(refused('code_repository_required: configure the codebase directory')[0]).toBe('other')
    expect(
      refused(
        'eval::start-validation failed: dependency error: eval::start-validation exceeded its 30000 ms timeout',
      )[0],
    ).toBe('other')
  })

  it('asks again unless the answer for these inputs is in hand or on its way', () => {
    const [a, b] = [asked(), asked({ candidate: 'other' })]
    expect(needsCheck({ phase: 'idle' }, a)).toBe(true)
    expect(needsCheck({ phase: 'checking', key: a }, a)).toBe(false)
    expect(needsCheck({ phase: 'checking', key: a }, b)).toBe(true)
    expect(needsCheck(resolvedAnswer(a, resolution), a)).toBe(false)
    expect(needsCheck(resolvedAnswer(a, resolution), b)).toBe(true)
    // After a refusal the person may have pushed: leaving the field asks again.
    expect(needsCheck(refusedAnswer(a, 'commit_not_pushed: the candidate commit'), a)).toBe(true)
  })

  it('opens Start only on a resolution of exactly the current inputs', () => {
    const [a, b] = [asked(), asked({ runs: 7 })]
    expect(startAllowed(resolvedAnswer(a, resolution), a)).toBe(true)
    expect(startAllowed(resolvedAnswer(a, resolution), b)).toBe(false)
    expect(startAllowed(resolvedAnswer(a, resolution), undefined)).toBe(false)
    expect(startAllowed({ phase: 'checking', key: a }, a)).toBe(false)
    expect(startAllowed(refusedAnswer(a, 'commit_not_pushed: the candidate commit'), a)).toBe(false)
    expect(startAllowed({ phase: 'idle' }, a)).toBe(false)
  })

  it('shows only the check that speaks for the inputs in the form', () => {
    const [a, b] = [asked(), asked({ candidate: 'other' })]
    const resolved = resolvedAnswer(a, resolution)
    expect(checkFor(resolved, a)).toBe(resolved)
    expect(checkFor(resolved, b)).toBeUndefined()
    expect(checkFor(resolved, undefined)).toBeUndefined()
    expect(checkFor({ phase: 'idle' }, a)).toBeUndefined()
  })

  it('drops an answer that arrives after typing moved on', () => {
    const [a, b] = [asked({ candidate: 'feat/fi' }), asked({ candidate: 'feat/fix' })]
    // The first call is in flight when the second one starts.
    let check: RefCheck = { phase: 'checking', key: a }
    check = { phase: 'checking', key: b }
    // The older answer lands first: it neither shows nor opens Start.
    check = settleCheck(check, refusedAnswer(a, 'git_ref_invalid: the candidate ref `feat/fi` is not a commit'))
    expect(check).toEqual({ phase: 'checking', key: b })
    expect(startAllowed(check, b)).toBe(false)
    // The current one settles it.
    check = settleCheck(check, resolvedAnswer(b, resolution))
    expect(startAllowed(check, b)).toBe(true)
    // An answer that arrives later still, for inputs already left, changes nothing.
    expect(settleCheck(check, refusedAnswer(a, 'commit_not_pushed: the candidate commit'))).toBe(check)
    // Neither does one nobody asked for.
    expect(settleCheck({ phase: 'idle' }, resolvedAnswer(a, resolution))).toEqual({ phase: 'idle' })
  })

  it('does not let an older answer open Start for inputs that went back and forth', () => {
    const [a, b] = [asked(), asked({ runs: 8 })]
    let check: RefCheck = { phase: 'checking', key: a }
    // The person changes the runs: the check for them starts, and the answer for the first inputs is no longer wanted.
    check = { phase: 'checking', key: b }
    check = settleCheck(check, resolvedAnswer(a, resolution))
    expect(startAllowed(check, a)).toBe(false)
    expect(startAllowed(check, b)).toBe(false)
  })
})
