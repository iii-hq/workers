import { describe, expect, it } from 'vitest'
import type { AnalysisRecord, Diagnostic, Suggestion } from '../../../types'
import {
  canCompare,
  diffSignals,
  diffSuggestions,
  keptNote,
  otherAnalyses,
  readsNote,
  relation,
  signalLabel,
  suggestionKey,
  turnRelation,
} from './compare'

function suggestion(title: string, overrides: Partial<Suggestion> = {}): Suggestion {
  return {
    title,
    observation: 'o',
    hypothesis: 'h',
    harness_component: 'Model notices',
    proposed_change: 'p',
    expected_effect: 'e',
    evidence: [],
    code_refs: [],
    limitations: 'l',
    validation: {
      scenario_id: 'tool_contract_recovery',
      reproduction: 'r',
      invariants: [],
      primary_metric: 'm',
      expectation: 'x',
      non_regression_controls: [],
    },
    ...overrides,
  }
}

const ref = { path: 'harness/src/registry_notice.rs', line_from: 120, line_to: 148 }

function diagnostic(fingerprint: string, target = 'state::get'): Diagnostic {
  return {
    rule_id: 'repeated_tool_error',
    rule_version: '1',
    fingerprint,
    session_id: 's',
    target,
    observation: 'o',
    correlation: 'unknown',
    evidence: [],
  }
}

function analysis(id: string, at: number, overrides: Partial<AnalysisRecord> = {}): AnalysisRecord {
  return {
    schema_version: 1,
    evaluation_id: id,
    observation_key: 'k',
    origin: 'manual',
    session_id: 's',
    turn_id: 't',
    model: { model: 'm', provider: 'p' },
    config_revision: 'r',
    rules_version: 'v',
    criteria_version: 'c',
    status: 'completed',
    step: 0,
    created_at: at,
    updated_at: at,
    deadline: at,
    observe_since: at,
    counters: { sessions: 1, entries: 1, diagnostics: 0, suggestions: 0, rejected_suggestions: 0, validations: 0 },
    stages: [],
    usage: { judge_calls: 0, judge_input_tokens: 0, judge_output_tokens: 0, judge_usage_complete: true },
    ...overrides,
  }
}

describe('suggestionKey', () => {
  it('ignores case, punctuation and spacing, never the Harness area', () => {
    expect(suggestionKey(suggestion('Scope the registry-changed notice!'))).toBe(
      suggestionKey(suggestion('  scope the REGISTRY changed   notice')),
    )
    expect(suggestionKey(suggestion('Scope it'))).not.toBe(
      suggestionKey(suggestion('Scope it', { harness_component: 'Context pruning' })),
    )
  })
})

describe('diffSuggestions', () => {
  it('finds the suggestions that stayed, went away and are new', () => {
    const before = [
      suggestion('Scope the notice'),
      suggestion('Keep retries from repeating', { harness_component: 'Context pruning' }),
    ]
    const now = [suggestion('scope the notice'), suggestion('Skip the notice when the contract is not called')]
    const diff = diffSuggestions(before, now)
    expect(diff.kept.map((kept) => [kept.before.index, kept.now.index])).toEqual([[1, 1]])
    expect(diff.dropped.map((placed) => [placed.index, placed.suggestion.title])).toEqual([
      [2, 'Keep retries from repeating'],
    ])
    expect(diff.added.map((placed) => [placed.index, placed.suggestion.title])).toEqual([
      [2, 'Skip the notice when the contract is not called'],
    ])
  })

  it('matches by meaning of the key, not by place: a suggestion that moved is still kept', () => {
    const diff = diffSuggestions([suggestion('A'), suggestion('B')], [suggestion('B'), suggestion('A')])
    expect(diff.kept.map((kept) => [kept.before.index, kept.now.index])).toEqual([
      [2, 1],
      [1, 2],
    ])
    expect(diff.dropped).toEqual([])
    expect(diff.added).toEqual([])
  })

  it('pairs two suggestions with one key off in order, and shows the leftover', () => {
    const diff = diffSuggestions([suggestion('Same'), suggestion('Same')], [suggestion('Same')])
    expect(diff.kept).toHaveLength(1)
    expect(diff.dropped.map((placed) => placed.index)).toEqual([2])
  })

  it('reads a reworded idea as one dropped and one new unless they cite the same evidence', () => {
    const diff = diffSuggestions([suggestion('Scope the notice')], [suggestion('Narrow the notice')])
    expect([diff.kept.length, diff.dropped.length, diff.added.length]).toEqual([0, 1, 1])
    const cites = (...ids: string[]) => ids.map((entry_id) => ({ session_id: 's', entry_id }))
    // Two entries of three shared: the same idea, said differently.
    const reworded = diffSuggestions(
      [suggestion('Scope the notice', { evidence: cites('a', 'b', 'c') })],
      [suggestion('Narrow the notice', { evidence: cites('b', 'c', 'd') })],
    )
    expect([reworded.kept.length, reworded.dropped.length, reworded.added.length]).toEqual([1, 0, 0])
    expect(reworded.kept[0].reworded).toBe(true)
    // One of three shared is not enough, nor is the same entry of another session.
    const apart = diffSuggestions(
      [suggestion('Scope the notice', { evidence: cites('a', 'b', 'c') })],
      [
        suggestion('Narrow the notice', {
          evidence: [...cites('c'), { session_id: 'other', entry_id: 'a' }, { session_id: 'other', entry_id: 'b' }],
        }),
      ],
    )
    expect([apart.kept.length, apart.dropped.length, apart.added.length]).toEqual([0, 1, 1])
  })

  it('pairs the reworded ones by the evidence they share, leaving the unrelated one alone (e256 against 9f83)', () => {
    const cites = (...ids: string[]) => ids.map((entry_id) => ({ session_id: 's', entry_id }))
    const earlier = [
      suggestion('Only send registry-change notices when a contract already in context actually changed', {
        evidence: cites('1', '2', '3', '4', '5', '6'),
      }),
      suggestion('Keep retries from repeating', { harness_component: 'Context pruning', evidence: cites('7', '8') }),
    ]
    const later = [
      suggestion('Only send the registry-changed notice for contracts the session actually holds', {
        evidence: cites('1', '2', '3', '4', 'x', 'y', 'z', 'w', 'v'),
      }),
    ]
    const diff = diffSuggestions(earlier, later)
    expect(diff.kept.map((kept) => [kept.before.index, kept.now.index, kept.reworded])).toEqual([[1, 1, true]])
    expect(diff.dropped.map((placed) => placed.index)).toEqual([2])
    expect(diff.added).toEqual([])
    expect(keptNote(diff.kept[0])).toMatch(/^Reworded, was “Only send registry-change notices/)
  })

  it('compares nothing with nothing', () => {
    expect(diffSuggestions([], [])).toEqual({ kept: [], dropped: [], added: [] })
  })
})

describe('notes', () => {
  it('says how the code references and the plan scenario changed for a kept suggestion', () => {
    const [kept] = diffSuggestions(
      [suggestion('Scope')],
      [suggestion('Scope', { code_refs: Array.from({ length: 8 }, () => ref) })],
    ).kept
    expect(keptNote(kept)).toBe('Code references 0 → 8 · plan scenario unchanged tool_contract_recovery')
    const changed = diffSuggestions(
      [suggestion('Scope')],
      [suggestion('Scope', { validation: { ...suggestion('x').validation, scenario_id: null } })],
    ).kept[0]
    expect(keptNote(changed)).toBe('Code references 0 → 0 · plan scenario tool_contract_recovery → new case needed')
  })

  it('says what a new suggestion read, or that it read nothing', () => {
    expect(readsNote(suggestion('x', { code_refs: [ref] }))).toBe('Reads harness/src/registry_notice.rs:120–148')
    expect(readsNote(suggestion('x'))).toBe('Reads no code')
  })
})

describe('diffSignals', () => {
  it('compares signals by fingerprint', () => {
    const diff = diffSignals(
      [diagnostic('fp1'), diagnostic('fp2', 'shell::exec')],
      [diagnostic('fp1'), diagnostic('fp3')],
    )
    expect(diff.kept).toBe(1)
    expect(diff.dropped.map((signal) => signal.fingerprint)).toEqual(['fp2'])
    expect(diff.added.map((signal) => signal.fingerprint)).toEqual(['fp3'])
  })

  it('labels a signal with its rule, target and fingerprint', () => {
    expect(signalLabel(diagnostic('sha256:2b90aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa9a4'))).toBe(
      'repeated_tool_error · state::get · fp 2b90…9a4',
    )
  })
})

describe('the analyses of a turn', () => {
  const rows = [analysis('a', 100), analysis('b', 300, { supersedes: 'a' }), analysis('c', 200, { status: 'failed' })]

  it('lists the others newest first', () => {
    expect(otherAnalyses(rows, 'b').map((record) => record.evaluation_id)).toEqual(['c', 'a'])
  })

  it('compares only what ran to the end', () => {
    expect(canCompare(rows[0])).toBe(true)
    expect(canCompare(rows[2])).toBe(false)
    expect(canCompare(analysis('d', 1, { status: 'cancelled' }))).toBe(false)
  })

  it('says which analysis replaced which', () => {
    expect(relation(rows[1], rows[0])).toBe('Replaced by this analysis')
    expect(relation(rows[0], rows[1])).toBe('Replaces this analysis')
    expect(relation(rows[1], rows[2])).toBeUndefined()
  })

  it('places the open analysis in the story of its turn', () => {
    expect(turnRelation(rows[1], rows)).toEqual({ label: 'Reanalysis of', evaluationId: 'a', at: 100 })
    // The replaced one finds its replacement among the others.
    expect(turnRelation(rows[0], rows)).toEqual({ label: 'Replaced by', evaluationId: 'b', at: 300 })
    expect(turnRelation(rows[2], rows)).toBeUndefined()
    // The turn is not read yet, or the earlier one is gone: the link stays, the time is unknown.
    expect(turnRelation(rows[1], null)).toEqual({ label: 'Reanalysis of', evaluationId: 'a', at: undefined })
    expect(turnRelation(rows[0], null)).toBeUndefined()
  })
})
