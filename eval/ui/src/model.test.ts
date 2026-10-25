import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { createEvalApi } from './api'
import { configFromDraft, draftFromConfig, draftFromJson, EMPTY_DRAFT } from './config-draft'
import {
  cycleTitle,
  fixFor,
  gaps,
  groupOf,
  marks,
  nowView,
  outcomeView,
  parseOpenContext,
  phaseOf,
  promotionSummary,
  readiness,
  sentinelResult,
  validateConfig,
} from './model'
import type { CycleDecision, CyclePlan, CycleRecord, EvidenceSide, LoopConfig, PromotionJournal } from './types'

const record = (patch: Partial<CycleRecord> = {}): CycleRecord => ({
  schema_version: 1,
  cycle_id: 'cycle_1',
  input: { objective: 'fewer repeated calls' },
  status: 'completed',
  stage: 'done',
  step: 3,
  created_at: 1,
  updated_at: 2,
  history: [{ at: 1, status: 'queued', stage: 'coverage', detail: 'queued' }],
  ...patch,
})

const side = (patch: Partial<EvidenceSide> = {}): EvidenceSide => ({
  execution_id: 'exec_b',
  runs: [{ run_id: 'r1', completed: true, technical: 'valid' }],
  n: 1,
  mean: 0.5,
  ...patch,
})

const decision = (patch: Partial<CycleDecision> = {}): CycleDecision => ({
  outcome: 'accepted',
  reason: 'pass rate rose by 20 pp',
  policy_revision: 'pol_1',
  baseline_execution_id: 'exec_b',
  candidate_execution_id: 'exec_c',
  comparability: { comparable: true, checks: [{ field: 'model', baseline: 'm', candidate: 'm', matches: true }] },
  primary: {
    scenario_id: 's',
    computed_at: 3,
    baseline: side(),
    candidate: side({ execution_id: 'exec_c', mean: 0.7 }),
    computed_outcome: 'validated_improvement',
    reason: 'moved',
  },
  preserved: [],
  decided_at: 4,
  ...patch,
})

const CONFIG: LoopConfig = {
  subject: { provider: 'anthropic', model: 'claude' },
  where: 'docker',
  stack: { name: 'harness', yaml: 'workers: []' },
  runs_per_side: 3,
  technical_retries: 1,
  max_scenarios: 2,
  estimated_usd_per_run: 0.5,
  min_effect: { tokens: 1000, pass_rate_pp: 10 },
  budget: { max_spend_usd: 20, max_duration_ms: 3_600_000, max_calls: 40, max_candidates: 2 },
  controls: ['minimal_path'],
}

describe('loop api', () => {
  it('calls only the loop functions with their exact payloads', async () => {
    const trigger = vi.fn().mockResolvedValue({ cycles: [], usage: {}, executions: [] })
    const api = createEvalApi({ iii: { trigger } } as unknown as Host)
    await api.config()
    await api.configure(CONFIG)
    await api.start({ objective: 'o' })
    await api.start({ execution_id: 'exec_1' })
    await api.list()
    await api.result('cycle_1')
    await api.cancel('cycle_1')
    await api.executions('')
    await api.executions('plan-1')
    expect(trigger.mock.calls.map(([id, payload]) => [id, payload])).toEqual([
      ['eval::config', {}],
      ['eval::configure', { config: CONFIG }],
      ['eval::start', { objective: 'o' }],
      ['eval::start', { execution_id: 'exec_1' }],
      ['eval::list', { limit: 200 }],
      ['eval::result', { cycle_id: 'cycle_1' }],
      ['eval::cancel', { cycle_id: 'cycle_1' }],
      ['eval::executions', {}],
      ['eval::executions', { query: 'plan-1' }],
    ])
  })
})

describe('outcome', () => {
  it('never reads a completed status as a decision, and accepted is approval only', () => {
    expect(outcomeView(record())).toMatchObject({ label: 'No decision' })
    expect(outcomeView(record({ status: 'waiting', stage: 'candidate' }))).toBeNull()
    const accepted = outcomeView(record({ decision: decision(), outcome: { code: 'decided', detail: 'd' } }))
    expect(accepted).toMatchObject({ label: 'Accepted', tone: 'ok' })
    // Without activation evidence it is never promoted or rolled back.
    expect(accepted?.label).not.toMatch(/promot|roll/i)
    expect(
      outcomeView(record({ decision: decision({ outcome: 'rejected', reason: 'lost pass rate' }) })),
    ).toMatchObject({
      label: 'Rejected',
      reason: 'lost pass rate',
    })
  })

  it('shows the recorded outcome code and failure reason', () => {
    expect(outcomeView(record({ outcome: { code: 'no_action', detail: 'nothing pertinent' } }))).toMatchObject({
      label: 'No action',
      reason: 'nothing pertinent',
    })
    expect(outcomeView(record({ outcome: { code: 'budget', detail: 'spend cap reached' } }))?.label).toBe(
      'Budget reached',
    )
    expect(outcomeView(record({ status: 'failed', failure: 'e2e unreachable' }))?.reason).toBe('e2e unreachable')
  })

  it('says what a waiting cycle waits for', () => {
    const waiting = record({
      status: 'waiting',
      stage: 'awaiting_execution',
      next: { action: 'await_execution', detail: 'baseline plan-9 runs', findings: [] },
      executions: [{ role: 'baseline', state: 'running', execution_id: 'plan-9' }],
    })
    expect(nowView(waiting)).toMatchObject({
      tone: 'accent',
      headline: "Measuring today's Harness on the E2E",
      detail: 'baseline plan-9 runs',
    })
  })
})

describe('gaps', () => {
  it('names coverage gaps, analysis unknowns and an unusable baseline', () => {
    const found = gaps(
      record({
        coverage: {
          need: 'unmapped',
          covered: false,
          selected: [],
          gaps: ['no test exercises provider fallback'],
          tests_total: 40,
          tests_revision: 'r',
          catalog_sha256: 'c',
          selected_at: 1,
        },
        analyses: [
          {
            execution_id: 'exec_1',
            admission_key: 'k',
            role: 'source',
            identity: {
              contract_digest: null,
              artifacts_digest: null,
              stack_digest: null,
              model: null,
              provider: null,
              harness_version: null,
              workers_revision: null,
              engine_version: null,
              e2e_revision: null,
            },
            findings: [
              { fingerprint: 'f', class: 'insufficient_evidence', signal: 'score_drop', observation: 'no error text' },
            ],
            unknowns: ['harness version not reported'],
            child_transcripts_complete: false,
            analyzed_at: 2,
          },
        ],
        baseline: { execution_id: 'exec_b', primary: side(), preserved: [], usable: false, reason: 'only 1 of 3 runs' },
        outcome: { code: 'no_coverage', detail: 'unmapped need' },
      }),
    )
    expect(found.map((gap) => [gap.kind, gap.reason])).toEqual([
      ['no_coverage', 'no test exercises provider fallback'],
      ['unknown', 'harness version not reported'],
      ['unknown', 'Not every child transcript was kept.'],
      ['inconclusive', 'no error text'],
      ['inconclusive', 'only 1 of 3 runs'],
    ])
  })

  it('names no_coverage, unknown and inconclusive in the compared evidence', () => {
    const found = gaps(
      record({
        decision: decision({
          outcome: 'inconclusive',
          reason: 'candidate unmeasured',
          comparability: { comparable: false, checks: [{ field: 'harness_version', baseline: 'v1', matches: false }] },
          primary: {
            scenario_id: 's',
            computed_at: 3,
            baseline: side({ mean: undefined }),
            candidate: side({
              execution_id: 'exec_c',
              runs: [{ run_id: 'r', completed: false, technical: 'docker' }],
              n: 0,
              mean: undefined,
            }),
            computed_outcome: 'inconclusive',
            reason: 'too few runs',
          },
        }),
      }),
    )
    expect(found.map((gap) => gap.kind)).toEqual(['unknown', 'unknown', 'no_coverage', 'inconclusive'])
    expect(found[2].reason).toContain('1 without a valid technical outcome')
  })

  it('has none for a clean decision', () => {
    expect(gaps(record({ decision: decision() }))).toEqual([])
  })
})

describe('loop config', () => {
  it('round-trips the stored config through the form draft', () => {
    expect(configFromDraft(draftFromConfig(CONFIG))).toEqual(CONFIG)
    expect(validateConfig(CONFIG)).toBeNull()
  })

  it('has no defaults: an empty draft saves nothing', () => {
    expect(configFromDraft(EMPTY_DRAFT)).toBeNull()
    const blank = configFromDraft({ ...EMPTY_DRAFT, where: 'harness' })
    expect(blank && validateConfig(blank)).toBe('subject.provider and subject.model must name a model')
  })

  it('applies the stack rule of the backend', () => {
    expect(validateConfig({ ...CONFIG, stack: undefined })).toMatch(/needs a stack/)
    expect(validateConfig({ ...CONFIG, where: 'harness' })).toMatch(/omit stack/)
    expect(validateConfig({ ...CONFIG, where: 'harness', stack: undefined })).toBeNull()
    expect(validateConfig({ ...CONFIG, budget: { ...CONFIG.budget, max_calls: 0 } })).toMatch(/budget/)
  })

  it('reads the advanced JSON editor back into the form', () => {
    expect(configFromDraft(draftFromJson(JSON.stringify(CONFIG)))).toEqual(CONFIG)
    expect(() => draftFromJson('{')).toThrow(/Not valid JSON/)
    expect(() => draftFromJson(JSON.stringify({ ...CONFIG, where: 'cloud' }))).toThrow(/where/)
  })
})

it('opens a cycle, the start form or the settings from the panel context', () => {
  expect(parseOpenContext({ type: 'cycle', cycleId: 'cycle_1' })).toEqual({ cycleId: 'cycle_1' })
  expect(parseOpenContext({ type: 'start' })).toEqual({ start: true })
  expect(parseOpenContext({ type: 'config' })).toEqual({ config: true })
  expect(parseOpenContext({ type: 'analysis' })).toBeNull()
})

describe('sentinel forward', () => {
  it('keeps only what Sentinel returned and never invents a triage', () => {
    expect(
      sentinelResult({
        dedupe_key: 'report:e:r:1:s:act:baseline:v:a:0',
        classification: 'unexpected',
        outcome: 'created',
        group_id: 'grp_1',
        occurrence_id: 'occ_1',
        group_status: 'new',
        triage: {
          kind: 'defect',
          confidence: 0.8,
          source: 'judge',
          model: 'jev-1',
          question: 'q',
          at_ms: 5,
          usage: { input_tokens: 120 },
        },
      }),
    ).toEqual({
      dedupe_key: 'report:e:r:1:s:act:baseline:v:a:0',
      classification: 'unexpected',
      outcome: 'created',
      group_id: 'grp_1',
      occurrence_id: 'occ_1',
      group_status: 'new',
      triage: {
        kind: 'defect',
        confidence: 0.8,
        source: 'judge',
        model: 'jev-1',
        question: 'q',
        criterion: undefined,
        at_ms: 5,
        usage: { input_tokens: 120, output_tokens: undefined },
      },
    })
    const untriaged = sentinelResult({ dedupe_key: 'k', classification: 'expected', outcome: 'deduped', triage: null })
    expect(untriaged?.triage).toBeUndefined()
    expect(sentinelResult({ outcome: 'created' })).toBeNull()
  })

  it('reports a failed forward as unknown, not as a finding', () => {
    const found = gaps(
      record({
        analyses: [
          {
            execution_id: 'exec_1',
            admission_key: 'k',
            role: 'source',
            identity: {
              contract_digest: null,
              artifacts_digest: null,
              stack_digest: null,
              model: null,
              provider: null,
              harness_version: null,
              workers_revision: null,
              engine_version: null,
              e2e_revision: null,
            },
            findings: [],
            unknowns: [],
            child_transcripts_complete: true,
            sentinel: { observations: 2, results: [], error: 'sentinel unreachable' },
            analyzed_at: 2,
          },
        ],
      }),
    )
    expect(found).toEqual([{ kind: 'unknown', subject: 'source · exec_1 · Sentinel', reason: 'sentinel unreachable' }])
  })
})

const STAGE2: LoopConfig = {
  ...CONFIG,
  judge: { provider: 'typesafe', estimated_usd: 0.02 },
  candidate: {
    repository: '/repo/workers',
    base_sha: 'a'.repeat(40),
    worktrees_dir: '/repo/.worktrees',
    model: 'claude',
    provider: 'anthropic',
    thinking_level: 'high',
    max_turns: 30,
    max_output_tokens: 20_000,
    max_total_tokens: 400_000,
    max_cost_usd: 5,
  },
  validation: {
    where: 'docker',
    stack_name: 'harness',
    source_stack_template: 'ref: commit:{commit}',
    artifact_stack_template: 'version: {version}',
  },
  release_build_estimate_usd: 1.5,
  promotion: {
    artifact_target: 'x86_64-unknown-linux-gnu',
    target: { engine_url: 'ws://127.0.0.1:49134', namespace: 'loop-test', compose_file: '/srv/worker-compose.yaml' },
  },
}

const running = {
  version: '1.2.0',
  pid: 42,
  binary_sha256: 'b'.repeat(64),
  file: 'f',
  namespace: 'n',
  operation_id: 'op',
}

describe('stage 2 config', () => {
  it('round-trips judge, candidate, validation and promotion through the form', () => {
    expect(configFromDraft(draftFromConfig(STAGE2))).toEqual(STAGE2)
    expect(validateConfig(STAGE2)).toBeNull()
  })

  it('applies the dependency rules of the backend', () => {
    expect(validateConfig({ ...STAGE2, candidate: undefined })).toMatch(/validation needs candidate/)
    expect(validateConfig({ ...STAGE2, validation: undefined })).toMatch(/promotion needs validation/)
    expect(validateConfig({ ...STAGE2, validation: { ...STAGE2.validation!, where: 'harness' } })).toMatch(
      /docker or github/,
    )
    expect(validateConfig({ ...STAGE2, candidate: { ...STAGE2.candidate!, base_sha: 'main' } })).toMatch(/40-hex/)
  })

  it('requires a positive release build estimate with promotion, and saves it only with promotion', () => {
    expect(validateConfig({ ...STAGE2, release_build_estimate_usd: undefined })).toMatch(/release_build_estimate_usd/)
    expect(validateConfig({ ...STAGE2, release_build_estimate_usd: 0 })).toMatch(/release_build_estimate_usd/)
    const off = draftFromConfig(STAGE2)
    off.sections = { ...off.sections, promotion: null }
    expect(configFromDraft(off)).not.toHaveProperty('release_build_estimate_usd')
  })

  it('has no service origins or credential names in the promotion config', () => {
    expect(Object.keys(STAGE2.promotion ?? {}).sort()).toEqual(['artifact_target', 'target'])
    const legacy = { ...STAGE2, promotion: { ...STAGE2.promotion, github_token_env: 'GITHUB_TOKEN' } }
    expect(() => draftFromJson(JSON.stringify(legacy))).toThrow(/promotion.github_token_env/)
    expect(() => draftFromJson(JSON.stringify({ ...STAGE2, registry_url: 'https://x' }))).toThrow(/registry_url/)
  })
})

describe('promotion', () => {
  const journal = (...phases: PromotionJournal['phases']): PromotionJournal => ({ promotion_id: 'p1', phases })
  const accepted = decision()

  it('keeps an accepted candidate Accepted until activation is confirmed and the canary held', () => {
    expect(outcomeView(record({ decision: accepted, outcome: { code: 'decided', detail: 'd' } }))?.label).toBe(
      'Accepted',
    )
    const requested = journal({ phase: 'activation_requested', operation_id: 'op', version: '1.2.0', accepted: true })
    expect(promotionSummary(requested).activation).toBe('requested')
    // The backend's claim alone is not enough.
    const claimed = outcomeView(
      record({ decision: accepted, outcome: { code: 'promoted', detail: 'canary held' }, promotion: requested }),
    )
    expect(claimed?.label).toBe('Promotion unconfirmed')
    const proved = journal(
      { phase: 'activation_requested', operation_id: 'op', version: '1.2.0', accepted: true },
      { phase: 'activation_confirmed', ...running },
    )
    const promotedRecord = (canary?: CycleRecord['canary']) =>
      record({ decision: accepted, outcome: { code: 'promoted', detail: 'activated' }, promotion: proved, canary })
    // Activation proved but the canary did not hold, or was not recorded: not promoted.
    expect(outcomeView(promotedRecord())).toMatchObject({ label: 'Promotion unconfirmed' })
    expect(outcomeView(promotedRecord({ execution_id: 'k', outcome: 'unknown', reason: 'r' }))?.reason).toMatch(
      /canary unknown/,
    )
    expect(outcomeView(promotedRecord({ execution_id: 'k', outcome: 'held', reason: 'r' }))?.label).toBe('Promoted')
  })

  it('says Rolled back only with the rollback confirmed', () => {
    const back = journal(
      { phase: 'activation_confirmed', ...running },
      { phase: 'rollback_requested', operation_id: 'op2', version: '1.1.0', accepted: true },
    )
    expect(
      outcomeView(record({ outcome: { code: 'rolled_back', detail: 'canary violated' }, promotion: back }))?.label,
    ).toBe('Rollback unconfirmed')
    const proved = journal(...back.phases, { phase: 'rollback_confirmed', ...running, version: '1.1.0' })
    expect(promotionSummary(proved).rollback).toBe('confirmed')
    expect(
      outcomeView(record({ outcome: { code: 'rolled_back', detail: 'canary violated' }, promotion: proved }))?.label,
    ).toBe('Rolled back')
  })

  it('marks a requested effect unknown when the promotion stopped unavailable or inconclusive', () => {
    const lost = journal(
      { phase: 'merge_requested', pull_request: 7, head_sha: 'c'.repeat(40), merge_sha: null },
      { phase: 'stopped', step: 'read_merge', kind: 'inconclusive', reason: 'merge response lost' },
    )
    expect(promotionSummary(lost)).toMatchObject({ merge: 'unknown', activation: 'not_requested' })
    expect(gaps(record({ promotion: lost }))).toContainEqual({
      kind: 'inconclusive',
      subject: 'Promotion \u00b7 read_merge (inconclusive)',
      reason: 'merge response lost',
    })
    const refused = journal({ phase: 'stopped', step: 'merge', kind: 'rejected', reason: 'head moved' })
    expect(gaps(record({ promotion: refused }))).toEqual([])
  })
})

describe('stage 2 record', () => {
  it('shows four phases that never move back, whatever stage path the cycle took', () => {
    const cycle = record({
      input: { execution_id: 'exec_1' },
      status: 'running',
      stage: 'coverage',
      history: [
        { at: 1, status: 'queued', stage: 'analysis', detail: 'admitted' },
        { at: 2, status: 'running', stage: 'triage', detail: 'judged' },
        { at: 4, status: 'running', stage: 'coverage', detail: 'mapping' },
      ],
    })
    expect(marks(cycle)).toEqual(['now', 'todo', 'todo', 'todo'])
    // A finished cycle is at `done`: the phase is where it stopped.
    const stopped = record({
      input: { execution_id: 'exec_1' },
      stage: 'done',
      outcome: { code: 'inconclusive', detail: 'a proposal needs a frozen plan and config.candidate' },
      config: { config: CONFIG, revision: 'rev', updated_at: 1 },
      history: [
        { at: 1, status: 'running', stage: 'proposal', detail: 'proposing' },
        { at: 2, status: 'completed', stage: 'done', detail: 'Inconclusive' },
      ],
    })
    expect(phaseOf(stopped)).toBe(1)
    expect(marks(stopped)).toEqual(['done', 'stop', 'todo', 'todo'])
    expect(groupOf(stopped, CONFIG)).toBe('you')
    expect(fixFor(stopped, CONFIG)).toBe('improve')
    expect(nowView(stopped, CONFIG).headline).toBe('Inconclusive: candidate work is off')
    // Once the current settings have it, there is nothing to turn on.
    const improving = { ...CONFIG, candidate: { repository: '/r' } } as LoopConfig
    expect(fixFor(stopped, improving)).toBeNull()
    expect(groupOf(stopped, improving)).toBe('done')
    const promoted = record({ decision: decision(), outcome: { code: 'decided', detail: 'accepted' } })
    expect(marks(promoted)).toEqual(['done', 'done', 'done', 'todo'])
    expect(groupOf(promoted, CONFIG)).toBe('done')
    expect(marks(record({ decision: decision({ outcome: 'rejected' }) }))).toEqual(['done', 'done', 'fail', 'todo'])
  })

  it('names an execution cycle by what its frozen plan restores', () => {
    const plan = {
      criterion: {
        metric: 'criterion_score',
        criterion_id: 'needles_recovered',
        direction: 'increase',
        min_effect: 10,
        min_runs: 5,
        scenario_id: 'context_pressure',
      },
    } as CyclePlan
    expect(cycleTitle(record({ input: { execution_id: 'plan-9' }, plan }))).toBe(
      'Restore needles_recovered on context_pressure',
    )
    expect(
      cycleTitle(
        record({
          input: { execution_id: 'plan-9' },
          plan: { ...plan, criterion: { ...plan.criterion, metric: 'pass_rate', criterion_id: undefined } },
        }),
      ),
    ).toBe('Restore pass rate on context_pressure')
    expect(cycleTitle(record({ input: { execution_id: 'plan-9' } }))).toBe('Execution plan-9')
    expect(cycleTitle(record())).toBe('fewer repeated calls')
  })

  it('says how far a new cycle can go with the settings', () => {
    expect(readiness(null)).toBe(0)
    expect(readiness(CONFIG)).toBe(1)
    const candidate = { ...CONFIG, candidate: {} } as LoopConfig
    expect(readiness(candidate)).toBe(2)
    expect(readiness({ ...candidate, validation: {} } as LoopConfig)).toBe(3)
    expect(readiness({ ...candidate, validation: {}, promotion: {} } as LoopConfig)).toBe(4)
  })

  it('reports an unknown JEV triage and failed or unmeasured actors without inventing results', () => {
    const found = gaps(
      record({
        triage: { source: 'jev', outcome: 'unknown', findings: ['f1'], reason: 'judge unavailable' },
        candidate: {
          identity: {
            candidate_id: 'c1',
            cycle_id: 'cycle_1',
            base_sha: 'a'.repeat(40),
            brief_digest: 'd',
            branch: 'loop/c1',
            worktree: '/w',
            root: '/w',
            session_id: 's',
            idempotency_key: 'k',
          },
          candidate: {
            identity: {
              candidate_id: 'c1',
              cycle_id: 'cycle_1',
              base_sha: 'a'.repeat(40),
              brief_digest: 'd',
              branch: 'loop/c1',
              worktree: '/w',
              root: '/w',
              session_id: 's',
              idempotency_key: 'k',
            },
            status: { state: 'failed', code: 'unsupported_isolation', reason: 'no fs scope' },
            turn_id: null,
            cost: { cost_usd: null, input_tokens: null, output_tokens: null },
            commit: null,
          },
        },
        canary: { execution_id: 'exec_k', outcome: 'unknown', reason: 'binary digest unknown' },
      }),
    )
    expect(found.map((gap) => [gap.kind, gap.subject])).toEqual([
      ['unknown', 'Triage \u00b7 jev'],
      ['inconclusive', 'Candidate'],
      ['unknown', 'Candidate cost'],
      ['unknown', 'Canary \u00b7 exec_k'],
    ])
  })
})
