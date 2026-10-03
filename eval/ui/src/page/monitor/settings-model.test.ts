import { describe, expect, it } from 'vitest'
import { LIMITS } from '../../fixtures'
import type { CatalogModel, MonitorConfig } from '../../types'
import {
  buildCatalog,
  CODE_ACCESS_RISK,
  codeDirectoryError,
  codeDirectoryHelp,
  codeRepository,
  draftFromConfig,
  isDirty,
  limitRows,
  modelKey,
  pickModel,
  runningHint,
  savedNotice,
  thinkingChoices,
  toMonitorModel,
  triageHint,
  triageWarning,
} from './settings-model'

const catalog: CatalogModel[] = [
  { id: 'deepseek-v4-pro', provider: 'deepseek', context_window: 1, max_output_tokens: 1, supports_thinking: true },
  {
    id: 'claude-sonnet-5-5',
    provider: 'anthropic',
    context_window: 1,
    max_output_tokens: 1,
    supports_thinking: true,
    supports_xhigh: true,
  },
  { id: 'claude-haiku-4-5', provider: 'anthropic', display_name: 'Haiku', context_window: 1, max_output_tokens: 1 },
  { id: 'claude-haiku-4-5', provider: 'anthropic', context_window: 1, max_output_tokens: 1 },
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
    expect(groups[0].options[1].description).toBe('thinking')
    expect(groups[0].options[0].description).toBeUndefined()
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
    const draft = { enabled: true, modelKey: sonnet?.key ?? null, thinking: 'xhigh' as const, codeDirectory: '' }
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
    expect(draftFromConfig(null)).toEqual({ enabled: false, modelKey: null, thinking: null, codeDirectory: '' })
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
      'Audit sample': '5% of sessions with no signal',
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
