import { describe, expect, it } from 'vitest'
import { COST, LIMITS } from '../../fixtures'
import type { AnalysisRecord, CatalogModel, MonitorConfig } from '../../types'
import {
  buildCatalog,
  CODE_ACCESS_RISK,
  COST_CAP_ERROR,
  capHelp,
  capProgress,
  ceilingLine,
  codeDirectoryError,
  codeDirectoryHelp,
  codeRepository,
  costCeiling,
  dailyCostCap,
  draftFromConfig,
  isDirty,
  lastWeekLine,
  limitRows,
  modelKey,
  modelPriceHint,
  notReportedLine,
  parseCostCap,
  perAnalysisHint,
  perAnalysisLine,
  pickModel,
  priceLine,
  priceText,
  runningHint,
  savedNotice,
  thinkingChoices,
  toMonitorModel,
  triageHint,
  triageWarning,
} from './settings-model'

const catalog: CatalogModel[] = [
  {
    id: 'deepseek-v4-pro',
    provider: 'deepseek',
    context_window: 1_000_000,
    max_output_tokens: 1,
    supports_thinking: true,
    pricing: { input: 1.32, output: 3.96 },
  },
  {
    id: 'claude-sonnet-5-5',
    provider: 'anthropic',
    context_window: 1_048_576,
    max_output_tokens: 1,
    supports_thinking: true,
    supports_xhigh: true,
    pricing: { input: 4, output: 20, cache_read: 0.4 },
  },
  {
    id: 'claude-haiku-4-5',
    provider: 'anthropic',
    display_name: 'Haiku',
    context_window: 200_000,
    max_output_tokens: 1,
  },
  { id: 'claude-haiku-4-5', provider: 'anthropic', context_window: 200_000, max_output_tokens: 1 },
]

const config = (patch: Partial<MonitorConfig> = {}): MonitorConfig => ({
  enabled: true,
  model: { model: 'claude-sonnet-5-5', provider: 'anthropic', thinking_level: 'medium' },
  revision: 'r1',
  updated_at: 1,
  ...patch,
})

describe('buildCatalog', () => {
  it('groups by provider, sorts, and drops duplicates', () => {
    const { groups } = buildCatalog(catalog, null)
    expect(groups.map((group) => group.label)).toEqual(['anthropic', 'deepseek'])
    expect(groups[0].options.map((option) => option.label)).toEqual([
      'claude-haiku-4-5 · anthropic',
      'claude-sonnet-5-5 · anthropic',
    ])
    expect(groups[0].options[1].description).toBe('$4 / $20 per Mtok · 1M ctx')
    expect(groups[1].options[0].description).toBe('$1.32 / $3.96 per Mtok · 1M ctx')
    // No price in the catalog: said so, never $0.
    expect(groups[0].options[0].description).toBe('Price not listed · 200k ctx')
    expect(groups[0].options[0].keywords).toContain('Haiku')
  })

  it('keeps the saved model when the catalog does not list it', () => {
    const saved = config().model
    const { byKey, groups } = buildCatalog([], saved)
    const entry = byKey.get(modelKey('anthropic', 'claude-sonnet-5-5'))
    expect(entry).toMatchObject({ listed: false, supportsThinking: true, supportsXhigh: false })
    expect(groups[0].options[0].description).toBe('Saved model, not in the catalog')
  })

  it('does not duplicate a listed saved model', () => {
    const { groups } = buildCatalog(catalog, config().model)
    expect(groups.flatMap((group) => group.options)).toHaveLength(3)
  })
})

describe('thinkingChoices and pickModel', () => {
  const { byKey } = buildCatalog(catalog, null)
  const sonnet = byKey.get(modelKey('anthropic', 'claude-sonnet-5-5'))
  const deepseek = byKey.get(modelKey('deepseek', 'deepseek-v4-pro'))
  const haiku = byKey.get(modelKey('anthropic', 'claude-haiku-4-5'))

  it('offers xhigh only when the model supports it, nothing without thinking', () => {
    expect(thinkingChoices(sonnet)).toEqual(['minimal', 'low', 'medium', 'high', 'xhigh'])
    expect(thinkingChoices(deepseek)).toEqual(['minimal', 'low', 'medium', 'high'])
    expect(thinkingChoices(haiku)).toEqual([])
    expect(thinkingChoices(undefined)).toEqual([])
  })

  it('keeps the level only if the new model accepts it', () => {
    const draft = {
      enabled: true,
      modelKey: sonnet?.key ?? null,
      thinking: 'xhigh' as const,
      codeDirectory: '',
      costCap: '',
    }
    expect(pickModel(draft, deepseek!).thinking).toBeNull()
    expect(pickModel({ ...draft, thinking: 'high' }, deepseek!).thinking).toBe('high')
    expect(pickModel({ ...draft, thinking: 'high' }, haiku!).thinking).toBeNull()
  })
})

describe('isDirty', () => {
  it('compares every field of the draft with the saved one', () => {
    const base = draftFromConfig(config())
    expect(isDirty(base, { ...base })).toBe(false)
    expect(isDirty(base, { ...base, enabled: false })).toBe(true)
    expect(isDirty(base, { ...base, thinking: 'high' })).toBe(true)
    expect(isDirty(base, { ...base, modelKey: modelKey('deepseek', 'deepseek-v4-pro') })).toBe(true)
    expect(isDirty(base, { ...base, codeDirectory: '/home/layon/workspaces/workers' })).toBe(true)
  })

  it('first run starts paused with nothing chosen', () => {
    expect(draftFromConfig(null)).toEqual({
      enabled: false,
      modelKey: null,
      thinking: null,
      codeDirectory: '',
      costCap: '',
    })
  })

  it('reads the saved directory, and ignores spaces around it', () => {
    const saved = draftFromConfig(config({ code_repository: '/home/layon/workspaces/workers' }))
    expect(saved.codeDirectory).toBe('/home/layon/workspaces/workers')
    expect(isDirty(saved, { ...saved, codeDirectory: ' /home/layon/workspaces/workers ' })).toBe(false)
    expect(isDirty(saved, { ...saved, codeDirectory: '' })).toBe(true)
    const none = draftFromConfig(config())
    expect(isDirty(none, { ...none, codeDirectory: '   ' })).toBe(false)
  })
})

describe('daily cost cap', () => {
  it('is empty for no cap, and an amount above zero otherwise', () => {
    expect(parseCostCap('')).toEqual({})
    expect(parseCostCap('   ')).toEqual({})
    expect(parseCostCap('5')).toEqual({ cap: 5 })
    expect(parseCostCap(' $5.00 ')).toEqual({ cap: 5 })
    expect(parseCostCap('.5')).toEqual({ cap: 0.5 })
    for (const typed of ['0', '0.0', '-1', 'five', '1e3', '5,00', '1.2.3', 'Infinity']) {
      expect(parseCostCap(typed), typed).toEqual({ error: COST_CAP_ERROR })
    }
  })

  it('travels with the draft: saved caps read back, and only a different amount is a change', () => {
    const saved = draftFromConfig(config({ daily_cost_cap_usd: 5 }))
    expect(saved.costCap).toBe('5')
    expect(draftFromConfig(config()).costCap).toBe('')
    expect(isDirty(saved, { ...saved, costCap: '5.00' })).toBe(false)
    expect(isDirty(saved, { ...saved, costCap: '7' })).toBe(true)
    expect(isDirty(saved, { ...saved, costCap: '' })).toBe(true)
    // An amount that cannot be read never equals what is saved.
    const none = draftFromConfig(config())
    expect(isDirty(none, { ...none, costCap: 'five' })).toBe(true)
  })

  it('sends the cap, or nothing to remove it', () => {
    const base = draftFromConfig(config())
    expect(dailyCostCap({ ...base, costCap: '2.5' })).toBe(2.5)
    expect(dailyCostCap({ ...base, costCap: '' })).toBeUndefined()
    expect(dailyCostCap({ ...base, costCap: 'nope' })).toBeUndefined()
  })

  it('shows how much of it today has used, and where the day ends on this machine', () => {
    const cost = {
      ...COST,
      since: new Date(2026, 9, 3, 21, 0).getTime() - 86_400_000,
      today_usd: 0.91,
      today_capture_usd: 0.91,
    }
    expect(capProgress(cost, 5)).toBe('$0.91 of $5.00 reported today.')
    expect(capProgress({ ...cost, today_usd: 4.31, today_replay_usd: 3.4 }, 5)).toBe(
      '$0.91 of $5.00 reported today. Not counted: replays $3.40.',
    )
    expect(capProgress(cost, undefined)).toBe('Empty means no cap.')
    expect(capHelp(cost.since)).toContain('The day is UTC: it ends at 21:00 on this machine.')
    expect(capHelp(cost.since)).toContain(
      "Analyses you start by hand still run. Unknown costs aren't counted, and neither are replays.",
    )
  })

  it('says in the notice what a save did to it', () => {
    const input = { runningCount: 0, now: 5_000_000, budgetMs: LIMITS.analysis_budget_ms }
    expect(savedNotice({ before: config(), after: config({ daily_cost_cap_usd: 5 }), ...input })).toBe(
      'New analyses use claude-sonnet-5-5 · medium. Automatic analyses pause for the day once $5.00 is reported. Observation is on.',
    )
    expect(savedNotice({ before: config({ daily_cost_cap_usd: 5 }), after: config(), ...input })).toBe(
      'New analyses use claude-sonnet-5-5 · medium. The daily cost cap is removed. Observation is on.',
    )
  })
})

describe('prices', () => {
  it('writes both prices per million tokens, trailing zeros trimmed, and never $0 for a missing price', () => {
    expect(priceText({ input: 15, output: 75 })).toBe('$15 / $75 per Mtok')
    expect(priceText({ input: 0.3, output: 1.2 })).toBe('$0.3 / $1.2 per Mtok')
    expect(priceText({ input: 1.32, output: 3.96 })).toBe('$1.32 / $3.96 per Mtok')
    expect(priceText(undefined)).toBe('Price not listed')
    expect(priceText({ input: 3 })).toBe('Price not listed')
  })

  it('adds the context window', () => {
    expect(priceLine({ pricing: { input: 15, output: 75 }, contextWindow: 200_000 })).toBe(
      '$15 / $75 per Mtok · 200k ctx',
    )
    expect(priceLine({ pricing: undefined, contextWindow: 1_000_000 })).toBe('Price not listed · 1M ctx')
    expect(priceLine({ pricing: undefined, contextWindow: undefined })).toBe('Price not listed')
  })

  it('says under the picker what the chosen model costs, and what thinking does to a bill', () => {
    const { byKey } = buildCatalog(catalog, config().model)
    const sonnet = byKey.get(modelKey('anthropic', 'claude-sonnet-5-5'))
    const haiku = byKey.get(modelKey('anthropic', 'claude-haiku-4-5'))
    expect(modelPriceHint(sonnet)).toBe('$4 / $20 per Mtok · 1M ctx. Higher thinking levels are slower and cost more.')
    expect(modelPriceHint(haiku)).toBe('Price not listed · 200k ctx.')
    // A saved model the catalog does not list has no price to state.
    expect(
      modelPriceHint(buildCatalog([], config().model).byKey.get(modelKey('anthropic', 'claude-sonnet-5-5'))),
    ).toBeNull()
    expect(modelPriceHint(undefined)).toBeNull()
  })
})

describe('cost section', () => {
  const stats = { count: 9, min: 0.04, median: 0.38, max: 1.27, unknown: 3 }

  it('states what an analysis has cost from the history, and says when there is none', () => {
    expect(perAnalysisLine(stats)).toBe('min $0.04 · median $0.38 · max $1.27')
    expect(perAnalysisHint(stats, 30)).toBe('9 analyses with a reported cost, last 30 days')
    expect(perAnalysisHint({ ...stats, count: 1 }, undefined)).toBe('1 analysis with a reported cost')
    expect(perAnalysisLine({ count: 0, unknown: 0 })).toBeNull()
    expect(perAnalysisHint({ count: 0, unknown: 0 }, 30)).toContain('Until then Reanalyze shows no estimate.')
  })

  it('gives a ceiling before the history can say anything: the token cap at the output price', () => {
    // 200,000 tokens at $20 per Mtok output; with code access 800,000.
    const pricing = { input: 4, output: 20 }
    expect(costCeiling(LIMITS, false, pricing)).toBe(4)
    expect(costCeiling(LIMITS, true, pricing)).toBe(16)
    // A price the catalog does not list is never read as free.
    expect(costCeiling(LIMITS, true, undefined)).toBeUndefined()
    expect(costCeiling(LIMITS, true, { input: 4 })).toBeUndefined()
    const none = { count: 0, unknown: 0 }
    expect(ceilingLine(16, none)).toBe('At most about $16.00')
    expect(ceilingLine(16, { ...stats, count: 2 })).toBe('At most about $16.00')
    // Three analyses with a reported cost: the history speaks, the ceiling goes.
    expect(ceilingLine(16, { ...stats, count: 3 })).toBeNull()
    expect(ceilingLine(undefined, none)).toBeNull()
  })

  it('counts the analyses that reported no cost apart, never as zero', () => {
    expect(notReportedLine(stats)).toBe('3 of 12 analyses')
    expect(notReportedLine({ ...stats, unknown: 0 })).toBeNull()
  })

  const DAY = 86_400_000
  const NOW = new Date(2026, 9, 3, 12, 0).getTime()
  const rec = (daysAgo: number, cost: number | undefined, ran = true) =>
    ({
      created_at: NOW - daysAgo * DAY,
      analyst: ran ? { session_id: 'a', sent_at: 0 } : undefined,
      usage: { llm_cost_usd: cost },
    }) as unknown as AnalysisRecord

  it('adds the last seven days from the list', () => {
    const records = [rec(1, 1.5), rec(2, 3.32), rec(3, undefined), rec(8, 9), rec(1, undefined, false)]
    expect(lastWeekLine(records, NOW, true)).toBe('$4.82 · 3 analyses · 1 not reported')
    expect(lastWeekLine(records, NOW, false)).toBe('at least $4.82 · 3 analyses · 1 not reported')
    expect(lastWeekLine([], NOW, true)).toBe('0 analyses')
    expect(lastWeekLine([rec(1, undefined)], NOW, true)).toBe('1 analysis · 1 not reported')
  })
})

describe('code directory', () => {
  it('sends it trimmed, and nothing when blank: that turns code access off', () => {
    const base = draftFromConfig(config())
    expect(codeRepository({ ...base, codeDirectory: '  /srv/workers ' })).toBe('/srv/workers')
    expect(codeRepository({ ...base, codeDirectory: '   ' })).toBeUndefined()
    expect(codeRepository(base)).toBeUndefined()
  })

  it('puts the backend refusals on the field, and only those', () => {
    expect(codeDirectoryError('code_repository must be an absolute path')).toBe(
      'Use an absolute path, for example /home/you/workspaces/workers',
    )
    expect(codeDirectoryError('invalid request: code_repository must be an absolute path')).toContain('absolute path')
    expect(codeDirectoryError('code_repository /home/layon/workspaces/wokers is not a directory')).toBe(
      'Not a directory: /home/layon/workspaces/wokers',
    )
    // Missing or not allowed is not "not a directory": the system's reason is shown.
    expect(
      codeDirectoryError('code_repository /home/layon/gone is not readable: No such file or directory (os error 2)'),
    ).toBe("Can't read /home/layon/gone: No such file or directory (os error 2)")
    expect(
      codeDirectoryError(
        'invalid request: code_repository /root/workers is not readable: Permission denied (os error 13)',
      ),
    ).toBe("Can't read /root/workers: Permission denied (os error 13)")
    expect(codeDirectoryError('code_repository /srv/x is not readable')).toBe("Can't read /srv/x")
    expect(codeDirectoryError('code_repository /srv/my dir/x is not a directory')).toBe(
      'Not a directory: /srv/my dir/x',
    )
    expect(codeDirectoryError('model claude-x is not in the router catalog')).toBeNull()
  })

  it('warns that the directory is no sandbox and the transcripts are untrusted', () => {
    expect(CODE_ACCESS_RISK).toContain('any function')
    expect(CODE_ACCESS_RISK).toContain('change files or start sessions')
    expect(CODE_ACCESS_RISK).toContain('untrusted transcripts')
    expect(CODE_ACCESS_RISK).toContain('does not confine absolute paths')
  })

  it('states the caps the monitor reports, and none before it has', () => {
    expect(codeDirectoryHelp(LIMITS)).toContain(
      'With it, an analysis may take up to 32 steps and 800k tokens. Leave it empty',
    )
    expect(codeDirectoryHelp(undefined)).toContain('not only the Harness. Leave it empty to keep code access off.')
    expect(codeDirectoryHelp(undefined)).not.toContain('steps')
  })
})

describe('toMonitorModel', () => {
  const { byKey } = buildCatalog(catalog, null)
  const sonnet = byKey.get(modelKey('anthropic', 'claude-sonnet-5-5'))!
  const deepseek = byKey.get(modelKey('deepseek', 'deepseek-v4-pro'))!

  it('omits an unset level', () => {
    expect(toMonitorModel(deepseek, null, null)).toEqual({ model: 'deepseek-v4-pro', provider: 'deepseek' })
  })

  it('keeps provider options only for the unchanged model', () => {
    const saved = { ...config().model, provider_options: { anthropic: { effort: 'x' } } }
    expect(toMonitorModel(sonnet, 'medium', saved)).toEqual(saved)
    expect(toMonitorModel(deepseek, null, saved)).not.toHaveProperty('provider_options')
  })
})

describe('savedNotice and runningHint', () => {
  const before = config({ updated_at: 1_000 })
  const budget = LIMITS.analysis_budget_ms
  const later = 1_000 + budget + 1_000
  const soon = 1_000 + 10_000

  it('says what new analyses use and whether observation is on', () => {
    expect(
      savedNotice({ before: null, after: config({ enabled: false }), runningCount: 0, now: later, budgetMs: budget }),
    ).toBe('New analyses use claude-sonnet-5-5 · medium. Observation is paused.')
  })

  it('names the directory the new analyses read, and says when it was cleared', () => {
    const withCode = config({ code_repository: '/home/layon/workspaces/workers' })
    const input = { runningCount: 0, now: later, budgetMs: budget }
    expect(savedNotice({ before: null, after: withCode, ...input })).toBe(
      'New analyses use claude-sonnet-5-5 · medium and read code in /home/layon/workspaces/workers. Observation is on.',
    )
    expect(savedNotice({ before: withCode, after: config(), ...input })).toBe(
      'New analyses use claude-sonnet-5-5 · medium and have no code access. Observation is on.',
    )
    expect(savedNotice({ before: config(), after: config(), ...input })).toBe(
      'New analyses use claude-sonnet-5-5 · medium. Observation is on.',
    )
  })

  it('names the old model only when no analysis can predate it', () => {
    const after = config({ model: { ...before.model, thinking_level: 'high' } })
    expect(savedNotice({ before, after, runningCount: 2, now: later, budgetMs: budget })).toBe(
      'New analyses use claude-sonnet-5-5 · high. 2 running analyses keep claude-sonnet-5-5 · medium. Observation is on.',
    )
    expect(savedNotice({ before, after, runningCount: 1, now: soon, budgetMs: budget })).toBe(
      'New analyses use claude-sonnet-5-5 · high. 1 running analysis keeps the model it started with. Observation is on.',
    )
  })

  it('says nothing about running analyses when the model did not change', () => {
    const paused = config({ enabled: false })
    expect(savedNotice({ before, after: paused, runningCount: 2, now: later, budgetMs: budget })).not.toContain('keep')
  })

  it('hints that running analyses keep their model', () => {
    expect(runningHint(0, before, later, budget)).toBeNull()
    expect(runningHint(2, before, later, budget)).toBe(
      '2 analyses are running with claude-sonnet-5-5 · medium. They keep it. A change here applies to new analyses only.',
    )
    expect(runningHint(1, before, soon, budget)).toBe(
      '1 analysis is running. It keeps the model it started with. A change here applies to new analyses only.',
    )
  })
})

describe('settled analyses', () => {
  const before = config({ updated_at: 1_000 })

  it('waits as long as the monitor says an analysis can run', () => {
    const after = config({ model: { ...before.model, thinking_level: 'high' } })
    const now = 1_000 + 181_000
    // 181 s is past a 180 s deadline, not past a 30 min one.
    expect(runningHint(1, before, now, 180_000)).toContain('running with claude-sonnet-5-5')
    expect(runningHint(1, before, now, LIMITS.analysis_budget_ms)).toBe(
      '1 analysis is running. It keeps the model it started with. A change here applies to new analyses only.',
    )
    expect(savedNotice({ before, after, runningCount: 1, now, budgetMs: LIMITS.analysis_budget_ms })).toContain(
      'keeps the model it started with',
    )
  })

  it('claims nothing settled before the deadline is known', () => {
    expect(runningHint(2, before, 1_000 + 10 ** 9, undefined)).toContain('They keep the model they started with')
  })
})

describe('limitRows', () => {
  it('states each limit the way the monitor reports it', () => {
    expect(Object.fromEntries(limitRows(LIMITS, false))).toEqual({
      Deadline: '30m 00s per analysis, queue included',
      Concurrency: '8 running · 500 not finished',
      'Context to models': '192.0 KiB of JSON',
      'Investigation steps': '1',
      'Investigation tokens': '200k total · 16,384 output',
      Retention: '30 days · 1,000 finished analyses',
    })
  })

  it('raises the investigation caps when a code directory is saved', () => {
    const rows = Object.fromEntries(limitRows(LIMITS, true))
    expect(rows['Investigation steps']).toBe('32')
    expect(rows['Investigation tokens']).toBe('800k total · 16,384 output')
  })

  it('follows the values, singular and plural', () => {
    const rows = Object.fromEntries(
      limitRows(
        {
          ...LIMITS,
          analysis_budget_ms: 90_000,
          investigation_max_total_tokens: 123_456,
          retention_days: 1,
          retention_max_terminal: 1,
        },
        false,
      ),
    )
    expect(rows.Deadline).toBe('1m 30s per analysis, queue included')
    expect(rows['Investigation tokens']).toBe('123,456 total · 16,384 output')
    expect(rows.Retention).toBe('1 day · 1 finished analysis')
  })
})

describe('triage copy', () => {
  it('tells the operator what to do per code', () => {
    expect(triageHint('missing_key')).toBe('Add the key in judge-typesafe settings.')
    expect(triageHint('provider_unavailable')).toBe('Start the judge and judge-typesafe workers.')
    expect(triageHint('unreachable')).toBe('Start the judge and judge-typesafe workers.')
    expect(triageHint('whatever')).toBe('Check judge-typesafe.')
    expect(triageHint(undefined)).toBe('Check judge-typesafe.')
  })

  it('warns that analyses fail at triage', () => {
    const triage = { provider: 'typesafe', available: false, code: 'missing_key', models: [], checked_at: 1 }
    expect(triageWarning(triage)).toMatch(/^Until the key is added, new analyses fail at triage/)
    expect(triageWarning({ ...triage, code: 'unreachable' })).toMatch(/^Until triage is available/)
  })
})
