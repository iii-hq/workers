import { describe, expect, it } from 'vitest'
import type {
  ContainerEntry,
  DeclaredContainer,
  EditPatch,
} from './compose-api'
import {
  alsoStarts,
  arrangeCheckouts,
  confirmStoredPatch,
  defaultRun,
  dependencyBlockReason,
  dependentsOf,
  draftFrom,
  entryShape,
  entryYaml,
  groupContainers,
  hasSettingsChanges,
  inPlaceVersion,
  isSecretValue,
  MASK,
  matchParts,
  parseLogLine,
  rebaseSettingsDraft,
  restoreSettingsDraft,
  settingsPatch,
  shortPath,
  startWaves,
  submittedSettingsEntry,
  usualParent,
  waveLabel,
} from './model'
import type { WorkerRow } from './types'

function declared(
  name: string,
  o: Partial<DeclaredContainer> = {},
): DeclaredContainer {
  return {
    name,
    source: 'path',
    ref: `/home/me/workspaces/workers/${name}`,
    version: null,
    start_after: [],
    environment: [],
    run: null,
    ...o,
  }
}

const project = [
  declared('state'),
  declared('llm-router', { start_after: ['state'] }),
  declared('provider-openai', { start_after: ['llm-router', 'state'] }),
  declared('harness', { start_after: ['provider-openai', 'state', 'ghost'] }),
  declared('database', {
    source: 'package',
    ref: 'api.workers.iii.dev/database',
    version: '0.5.17',
  }),
  declared('harness-e2e', {
    ref: '/home/me/.codex/worktrees/scenario-editorial/harness-e2e',
  }),
]
const byName = new Map(project.map((d) => [d.name, d]))

describe('groupContainers', () => {
  const containers = [
    { container: 'state', state: 'ready' },
    { container: 'database', state: 'ready' },
    {
      container: 'provider-openai',
      state: 'failed',
      last_error: 'exited with status 101',
    },
    { container: 'harness-e2e', state: 'ready' },
    { container: 'orphan', state: 'stopped' },
  ]

  it('puts failures first, then registry, local path and undeclared containers', () => {
    const groups = groupContainers(containers, byName)
    expect(groups.map((g) => [g.id, g.items.map((i) => i.name)])).toEqual([
      ['attention', ['provider-openai']],
      ['registry', ['database']],
      ['path', ['state', 'harness-e2e']],
      ['other', ['orphan']],
    ])
  })

  it('describes each row by what tells it apart', () => {
    const items = groupContainers(containers, byName).flatMap((g) => g.items)
    const detail = Object.fromEntries(items.map((i) => [i.name, i.detail]))
    expect(detail).toMatchObject({
      'provider-openai': 'exited with status 101',
      database: '0.5.17',
      state: '~/workspaces/workers/state',
      'harness-e2e': '~/.codex/worktrees/scenario-editorial/harness-e2e',
    })
  })

  it('keeps relative worker paths useful instead of repeating their parent', () => {
    const groups = groupContainers(
      [{ container: 'http', state: 'ready' }],
      new Map([['http', declared('http', { ref: '../http' })]]),
    )
    expect(groups[0].items[0].detail).toBe('../http')
  })

  it('lists engine workers compose does not run, and only those', () => {
    const row = (name: string, o: Partial<WorkerRow>): WorkerRow => ({
      id: name,
      name,
      runtime: 'node',
      ipAddress: null,
      version: '1.0.0',
      pid: 7,
      tag: null,
      managementKind: 'standalone',
      status: 'connected',
      stopEnabled: false,
      stopDisabledReason: null,
      composeState: null,
      lastError: null,
      ...o,
    })
    const groups = groupContainers(containers, byName, '', [
      row('database', { managementKind: 'compose' }),
      row('by-hand', {}),
      row('supervised', {
        managementKind: 'supervisor',
        status: 'stopped',
        runtime: null,
        version: null,
      }),
    ])
    const outside = groups.find((g) => g.id === 'outside')
    expect(outside?.items.map((i) => [i.name, i.state, i.detail])).toEqual([
      ['by-hand', 'ready', 'standalone · node · 1.0.0'],
      ['supervised', 'stopped', 'managed'],
    ])
  })

  it('filters by name and drops empty groups', () => {
    expect(
      groupContainers(containers, byName, 'DATA').map((g) => g.id),
    ).toEqual(['registry'])
  })
})

describe('startWaves', () => {
  it('orders containers by their deepest start_after, ignoring undeclared names', () => {
    expect(startWaves(project)).toEqual([
      ['state', 'database', 'harness-e2e'],
      ['llm-router'],
      ['provider-openai'],
      ['harness'],
    ])
  })

  it('survives a cycle instead of recursing forever', () => {
    const cycle = [
      declared('a', { start_after: ['b'] }),
      declared('b', { start_after: ['a'] }),
    ]
    expect(startWaves(cycle).flat().sort()).toEqual(['a', 'b'])
  })
})

describe('small helpers', () => {
  it('lists dependents', () => {
    expect(dependentsOf(project, 'state')).toEqual([
      'llm-router',
      'provider-openai',
      'harness',
    ])
  })

  it('shortens home directories', () => {
    expect(shortPath('/home/me/workspaces/x')).toBe('~/workspaces/x')
    expect(shortPath('/Users/me')).toBe('~')
    expect(shortPath('/opt/x')).toBe('/opt/x')
  })

  it('finds the folder most local workers live in', () => {
    expect(
      usualParent(['/home/me/w/a', '/home/me/w/b', '/home/me/.codex/wt/c']),
    ).toBe('/home/me/w')
    expect(usualParent([])).toBeNull()
  })
})

describe('dependencyBlockReason', () => {
  it('explains why a worker cannot wait for itself', () => {
    expect(dependencyBlockReason([declared('a')], 'a', 'a')).toBe(
      'Would create a startup cycle: a → a.',
    )
  })

  it('blocks direct and transitive cycles while allowing independent workers', () => {
    const containers = [
      declared('a'),
      declared('b', { start_after: ['a'] }),
      declared('c', { start_after: ['b'] }),
      declared('independent'),
    ]
    expect(dependencyBlockReason(containers, 'a', 'b')).toBe(
      'Would create a startup cycle: a → b → a.',
    )
    expect(dependencyBlockReason(containers, 'a', 'c')).toBe(
      'Would create a startup cycle: a → c → b → a.',
    )
    expect(dependencyBlockReason(containers, 'a', 'independent')).toBeNull()
  })

  it('terminates on existing cycles and still finds a branch reaching the worker', () => {
    const containers = [
      declared('a'),
      declared('x', { start_after: ['y'] }),
      declared('y', { start_after: ['x'] }),
      declared('z', { start_after: ['x', 'a'] }),
    ]
    expect(dependencyBlockReason(containers, 'a', 'x')).toBeNull()
    expect(dependencyBlockReason(containers, 'a', 'z')).toBe(
      'Would create a startup cycle: a → z → a.',
    )
  })

  it('supports object prototype names without changing the source graph', () => {
    const containers = [
      declared('__proto__'),
      declared('constructor', { start_after: ['__proto__'] }),
      declared('toString', { start_after: ['constructor'] }),
    ]
    const before = structuredClone(containers)
    expect(dependencyBlockReason(containers, '__proto__', 'toString')).toBe(
      'Would create a startup cycle: __proto__ → toString → constructor → __proto__.',
    )
    expect(containers).toEqual(before)
  })
})

const entry: ContainerEntry = {
  name: 'harness-e2e',
  worker: 'path:///home/me/wt/harness-e2e',
  version: null,
  start_after: [],
  env_file: ['.env'],
  environment: [
    { key: 'RUST_LOG', value: 'info', secret: false },
    { key: 'OPENAI_API_KEY', value: null, secret: true },
  ],
  run: './target/debug/harness-e2e',
  config_override: 'data_dir: ~/.iii/data/harness-e2e\n',
}

describe('settingsPatch', () => {
  it('is empty until something changes', () => {
    expect(settingsPatch(entry, draftFrom(entry))).toEqual({
      patch: {},
      changes: 0,
    })
  })

  it('sends only the changed fields, never an untouched secret', () => {
    const draft = draftFrom(entry)
    draft.run = 'cargo run --locked --bin harness-e2e '
    draft.startAfter = ['state']
    draft.env[0].value = 'debug'
    draft.env.push({
      key: 'NEW',
      value: '1',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    expect(settingsPatch(entry, draft)).toEqual({
      patch: {
        run: 'cargo run --locked --bin harness-e2e',
        start_after: ['state'],
        environment: { set: { RUST_LOG: 'debug', NEW: '1' }, unset: [] },
      },
      changes: 4,
    })
  })

  it('replaces a secret only once a new value is typed, and removes by key', () => {
    const draft = draftFrom(entry)
    draft.env[1].replacing = true
    expect(() => settingsPatch(entry, draft)).toThrow(/value|replacement/i)
    draft.env[1].value = 'sk-new'
    draft.env[0].removed = true
    expect(settingsPatch(entry, draft).patch.environment).toEqual({
      set: { OPENAI_API_KEY: 'sk-new' },
      unset: ['RUST_LOG'],
    })
  })

  it('rejects duplicate new keys instead of overwriting the first value', () => {
    const draft = draftFrom(entry)
    draft.env.push(
      {
        key: 'PORT',
        value: '3000',
        secret: false,
        replacing: false,
        removed: false,
        isNew: true,
      },
      {
        key: 'PORT',
        value: '4000',
        secret: false,
        replacing: false,
        removed: false,
        isNew: true,
      },
    )
    expect(() => settingsPatch(entry, draft)).toThrow(/duplicate|already/i)
  })

  it('ignores a removed new row instead of setting its value', () => {
    const draft = draftFrom(entry)
    draft.env.push({
      key: 'PORT',
      value: '3000',
      secret: false,
      replacing: false,
      removed: true,
      isNew: true,
    })
    expect(settingsPatch(entry, draft)).toEqual({ patch: {}, changes: 0 })
  })

  it('preserves a valid environment name that matches an object prototype key', () => {
    const draft = draftFrom(entry)
    draft.env.push({
      key: '__proto__',
      value: 'literal',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const result = settingsPatch(entry, draft)
    expect(result.changes).toBe(1)
    expect(JSON.stringify(result.patch.environment?.set)).toBe(
      '{"__proto__":"literal"}',
    )
  })
})

describe('hasSettingsChanges', () => {
  it('recognizes an unfinished new row as pending work', () => {
    const draft = draftFrom(entry)
    expect(hasSettingsChanges(entry, draft)).toBe(false)
    draft.env.push({
      key: '',
      value: '',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    expect(hasSettingsChanges(entry, draft)).toBe(true)
    draft.env.pop()
    expect(hasSettingsChanges(entry, draft)).toBe(false)
  })

  it('recognizes pending secret replacement and returns clean after cancellation', () => {
    const draft = draftFrom(entry)
    draft.env[1].replacing = true
    expect(hasSettingsChanges(entry, draft)).toBe(true)
    draft.env[1].replacing = false
    expect(hasSettingsChanges(entry, draft)).toBe(false)
  })

  it('keeps invalid config edits pending and ignores normalized no-op fields', () => {
    const draft = draftFrom(entry)
    draft.run += ' '
    draft.config += '\n'
    expect(hasSettingsChanges(entry, draft)).toBe(false)
    draft.config = 'port: ['
    expect(hasSettingsChanges(entry, draft)).toBe(true)
  })
})

describe('restoreSettingsDraft', () => {
  it('restores known settings by key while retaining the current patch and discard baseline', () => {
    const current: ContainerEntry = {
      ...entry,
      run: 'new-command',
      start_after: ['new-worker'],
      config_override: 'port: 4000\n',
      environment: [
        { key: 'MODE', value: 'production', secret: false },
        { key: 'RECENT', value: 'new', secret: false },
        { key: 'API_KEY', value: null, secret: true },
      ],
    }
    const previous: ContainerEntry = {
      ...entry,
      worker: 'path:///previous-location',
      version: 'old-version',
      env_file: ['previous.env'],
      run: 'old-command',
      start_after: ['old-worker'],
      config_override: 'port: 3000\n',
      environment: [
        { key: 'REMOVED', value: 'restore-me', secret: false },
        { key: 'API_KEY', value: null, secret: true },
        { key: 'MODE', value: 'development', secret: false },
      ],
    }
    const { draft, unavailableSecrets } = restoreSettingsDraft(
      current,
      previous,
    )
    expect(draft.env.map((row) => row.key)).toEqual([
      'MODE',
      'RECENT',
      'API_KEY',
      'REMOVED',
    ])
    expect(settingsPatch(current, draft)).toEqual({
      changes: 6,
      patch: {
        run: 'old-command',
        start_after: ['old-worker'],
        config_override: 'port: 3000\n',
        environment: {
          set: { MODE: 'development', REMOVED: 'restore-me' },
          unset: ['RECENT'],
        },
      },
    })
    expect(unavailableSecrets).toEqual(['API_KEY'])
    expect(hasSettingsChanges(current, draft)).toBe(true)
    expect(hasSettingsChanges(current, draftFrom(current))).toBe(false)
    expect(entryShape(current, draft)).toMatchObject({
      worker: current.worker,
      version: current.version,
      envFile: current.env_file,
    })
  })

  it('reports a removed previous secret without inventing its value', () => {
    const current = { ...entry, environment: [] }
    const previous = {
      ...entry,
      environment: [{ key: 'REMOVED_TOKEN', value: null, secret: true }],
    }
    const { draft, unavailableSecrets } = restoreSettingsDraft(
      current,
      previous,
    )
    expect(draft.env).toEqual([])
    expect(unavailableSecrets).toEqual(['REMOVED_TOKEN'])
    expect(settingsPatch(current, draft).patch).toEqual({})
  })

  it('preserves current credentials that did not exist previously', () => {
    const previous = { ...entry, environment: [] }
    const { draft } = restoreSettingsDraft(entry, previous)
    expect(draft.env[0].removed).toBe(true)
    expect(draft.env[1]).toEqual(draftFrom(entry).env[1])
    expect(settingsPatch(entry, draft).patch.environment).toEqual({
      set: {},
      unset: ['RUST_LOG'],
    })
  })

  it('preserves credentials when either side has an unavailable literal secret', () => {
    const current: ContainerEntry = {
      ...entry,
      environment: [
        { key: 'CURRENT_TOKEN', value: null, secret: true },
        // biome-ignore lint/suspicious/noTemplateCurlyInString: Compose environment references are literal strings.
        { key: 'PREVIOUS_TOKEN', value: '${TOKEN}', secret: false },
      ],
    }
    const previous: ContainerEntry = {
      ...entry,
      environment: [
        // biome-ignore lint/suspicious/noTemplateCurlyInString: Compose environment references are literal strings.
        { key: 'CURRENT_TOKEN', value: '${TOKEN}', secret: false },
        { key: 'PREVIOUS_TOKEN', value: null, secret: true },
      ],
    }
    const { draft, unavailableSecrets } = restoreSettingsDraft(
      current,
      previous,
    )
    expect(draft.env).toEqual(draftFrom(current).env)
    expect(unavailableSecrets).toEqual(['PREVIOUS_TOKEN'])
    expect(settingsPatch(current, draft).patch).toEqual({})
  })

  it('restores an empty nonsecret value without mistaking null for a secret', () => {
    const current = {
      ...entry,
      environment: [{ key: 'EMPTY', value: 'filled', secret: false }],
    }
    const previous = {
      ...entry,
      environment: [{ key: 'EMPTY', value: null, secret: false }],
    }
    const { draft } = restoreSettingsDraft(current, previous)
    expect(settingsPatch(current, draft).patch.environment).toEqual({
      set: { EMPTY: '' },
      unset: [],
    })
  })

  it('never writes a mask or a historical credential with inconsistent metadata', () => {
    const current = {
      ...entry,
      environment: [{ key: 'EXISTING_MASK', value: MASK, secret: false }],
    }
    const previous = {
      ...entry,
      environment: [
        { key: 'HISTORICAL_MASK', value: MASK, secret: false },
        { key: 'API_KEY', value: 'old-literal', secret: false },
      ],
    }
    const { draft, unavailableSecrets } = restoreSettingsDraft(
      current,
      previous,
    )
    expect(draft.env).toEqual(draftFrom(current).env)
    expect(unavailableSecrets).toEqual(['HISTORICAL_MASK', 'API_KEY'])
    expect(settingsPatch(current, draft).patch).toEqual({})
  })

  it('copies restored arrays and rows without changing either saved entry', () => {
    const current = structuredClone(entry)
    const previous = structuredClone(entry)
    previous.start_after = ['state']
    const before = structuredClone({ current, previous })
    const { draft } = restoreSettingsDraft(current, previous)
    draft.startAfter.push('later')
    draft.env[0].value = 'trace'
    expect({ current, previous }).toEqual(before)
    const unchanged = restoreSettingsDraft(current, current).draft
    expect(hasSettingsChanges(current, unchanged)).toBe(false)
    expect(settingsPatch(current, unchanged).patch).toEqual({})
  })
})

describe('submittedSettingsEntry', () => {
  it('retains written values as a clean baseline without pretending the old readback changed', () => {
    const submitted = submittedSettingsEntry(entry, {
      run: 'cargo run --bin harness-e2e',
      start_after: ['state'],
      config_override: 'port: 4000\n',
    })
    const retained = draftFrom(submitted)
    expect(submitted).toMatchObject({
      run: 'cargo run --bin harness-e2e',
      start_after: ['state'],
      config_override: 'port: 4000\n',
    })
    expect(hasSettingsChanges(entry, retained)).toBe(true)
    expect(hasSettingsChanges(submitted, retained)).toBe(false)
    expect(settingsPatch(submitted, retained)).toEqual({
      patch: {},
      changes: 0,
    })
    retained.run = 'another-command'
    expect(hasSettingsChanges(submitted, retained)).toBe(true)
    expect(settingsPatch(submitted, retained)).toEqual({
      patch: { run: 'another-command' },
      changes: 1,
    })
    expect(hasSettingsChanges(submitted, draftFrom(submitted))).toBe(false)
  })

  it('masks submitted credentials and never replays them on a later unrelated save', () => {
    const submitted = submittedSettingsEntry(entry, {
      environment: {
        set: { OPENAI_API_KEY: 'sk-written', NEW_TOKEN: 'new-written' },
        unset: [],
      },
    })
    expect(submitted.environment).toEqual([
      { key: 'RUST_LOG', value: 'info', secret: false },
      { key: 'OPENAI_API_KEY', value: null, secret: true },
      { key: 'NEW_TOKEN', value: null, secret: true },
    ])
    const next = draftFrom(submitted)
    expect(next.env.every((row) => !row.replacing && !row.isNew)).toBe(true)
    next.env[0].value = 'debug'
    expect(settingsPatch(submitted, next).patch.environment).toEqual({
      set: { RUST_LOG: 'debug' },
      unset: [],
    })
    next.env[1].replacing = true
    next.env[1].value = 'sk-newer'
    expect(settingsPatch(submitted, next).patch.environment).toEqual({
      set: { RUST_LOG: 'debug', OPENAI_API_KEY: 'sk-newer' },
      unset: [],
    })
  })

  it('rebases environment additions and removals so only newer edits enter the next patch', () => {
    const submitted = submittedSettingsEntry(entry, {
      environment: { set: { NEW_VAR: 'written' }, unset: ['RUST_LOG'] },
    })
    expect(submitted.environment.map((row) => row.key)).toEqual([
      'OPENAI_API_KEY',
      'NEW_VAR',
    ])
    const next = draftFrom(submitted)
    next.env[1].value = 'newer'
    expect(settingsPatch(submitted, next).patch.environment).toEqual({
      set: { NEW_VAR: 'newer' },
      unset: [],
    })
  })

  it('copies the saved snapshot and patch arrays without mutating either source', () => {
    const current = structuredClone(entry)
    const patch: EditPatch = {
      start_after: ['state'],
      environment: { set: { NEW_VAR: 'written' }, unset: [] },
    }
    const before = structuredClone({ current, patch })
    const submitted = submittedSettingsEntry(current, patch)
    submitted.start_after.push('later')
    submitted.env_file.push('later.env')
    submitted.environment[0].value = 'debug'
    expect({ current, patch }).toEqual(before)
  })

  it('preserves empty and referenced values while removing any masked or literal credential value', () => {
    const current = {
      ...entry,
      environment: [
        { key: 'LEGACY_TOKEN', value: 'legacy-literal', secret: false },
        { key: 'LEGACY_MASK', value: MASK, secret: false },
      ],
    }
    const submitted = submittedSettingsEntry(current, {
      environment: {
        // biome-ignore lint/suspicious/noTemplateCurlyInString: Compose environment references are literal strings.
        set: { EMPTY: '', REFERENCE_TOKEN: '${TOKEN}', PLACEHOLDER: MASK },
        unset: [],
      },
    })
    expect(submitted.environment).toEqual([
      { key: 'LEGACY_TOKEN', value: null, secret: true },
      { key: 'LEGACY_MASK', value: null, secret: true },
      { key: 'EMPTY', value: null, secret: false },
      // biome-ignore lint/suspicious/noTemplateCurlyInString: Compose environment references are literal strings.
      { key: 'REFERENCE_TOKEN', value: '${TOKEN}', secret: false },
      { key: 'PLACEHOLDER', value: null, secret: true },
    ])
    const next = draftFrom(submitted)
    next.run = 'later-command'
    expect(settingsPatch(submitted, next).patch).toEqual({
      run: 'later-command',
    })
  })

  it('keeps new invalid edits pending instead of treating the whole retained draft as saved', () => {
    const submitted = submittedSettingsEntry(entry, { run: 'written-command' })
    const next = draftFrom(submitted)
    next.config = 'invalid: ['
    expect(hasSettingsChanges(submitted, next)).toBe(true)
    expect(() => settingsPatch(submitted, next)).toThrow(/YAML/)
    expect(settingsPatch(submitted, draftFrom(submitted)).patch).toEqual({})
  })
})

describe('rebaseSettingsDraft', () => {
  it('keeps newer edits and accepts readback values for fields the user did not change', () => {
    const baseline = submittedSettingsEntry(entry, {
      run: 'written-command',
      config_override: 'port: 4000\n',
    })
    const pending = draftFrom(baseline)
    pending.run = 'newer-command'
    const loaded = {
      ...baseline,
      start_after: ['state'],
      config_override: 'port: 4000',
    }
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    expect(rebased).toMatchObject({
      run: 'newer-command',
      startAfter: ['state'],
      config: 'port: 4000',
    })
    expect(settingsPatch(loaded, rebased).patch).toEqual({
      run: 'newer-command',
    })
  })

  it('realigns newer environment edits by key when readback changes row order', () => {
    const baseline = submittedSettingsEntry(entry, {
      environment: { set: { NEW_VAR: 'written' }, unset: [] },
    })
    const pending = draftFrom(baseline)
    pending.env[0].value = 'debug'
    pending.env[1].replacing = true
    pending.env[1].value = 'sk-unwritten'
    const loaded = {
      ...baseline,
      environment: [
        baseline.environment[2],
        baseline.environment[1],
        baseline.environment[0],
      ],
    }
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    expect(rebased.env.map((row) => row.key)).toEqual([
      'NEW_VAR',
      'OPENAI_API_KEY',
      'RUST_LOG',
    ])
    expect(settingsPatch(loaded, rebased).patch.environment).toEqual({
      set: { OPENAI_API_KEY: 'sk-unwritten', RUST_LOG: 'debug' },
      unset: [],
    })
  })

  it('retains unfinished variables and invalid YAML as unsaved edits after readback', () => {
    const baseline = submittedSettingsEntry(entry, { run: 'written-command' })
    const pending = draftFrom(baseline)
    pending.config = 'invalid: ['
    pending.env.push({
      key: '',
      value: 'unfinished',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const loaded = structuredClone(baseline)
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    expect(rebased.config).toBe('invalid: [')
    expect(rebased.env.at(-1)).toEqual(pending.env.at(-1))
    expect(hasSettingsChanges(loaded, rebased)).toBe(true)
    expect(() => settingsPatch(loaded, rebased)).toThrow()
  })

  it('keeps duplicate new rows and empty secret replacements invalid', () => {
    const baseline = submittedSettingsEntry(entry, { run: 'written-command' })
    const pending = draftFrom(baseline)
    pending.env[1].replacing = true
    pending.env.push({
      key: 'RUST_LOG',
      value: 'duplicate',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const loaded = structuredClone(baseline)
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    expect(rebased.env[1]).toMatchObject({ replacing: true, value: '' })
    expect(rebased.env.at(-1)).toEqual(pending.env.at(-1))
    expect(hasSettingsChanges(loaded, rebased)).toBe(true)
    expect(() => settingsPatch(loaded, rebased)).toThrow()
  })

  it('recognizes a new readable value already present in readback without creating a duplicate', () => {
    const pending = draftFrom(entry)
    pending.env.push({
      key: 'NEW_VAR',
      value: 'written',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const loaded = submittedSettingsEntry(entry, {
      environment: { set: { NEW_VAR: 'written' }, unset: [] },
    })
    const rebased = rebaseSettingsDraft(entry, pending, loaded)
    expect(rebased.env.filter((row) => row.key === 'NEW_VAR')).toHaveLength(1)
    expect(hasSettingsChanges(loaded, rebased)).toBe(false)
    expect(settingsPatch(loaded, rebased).patch).toEqual({})
  })

  it('retains an unconfirmed new secret as a replacement when readback masks its value', () => {
    const pending = draftFrom(entry)
    pending.env.push({
      key: 'NEW_TOKEN',
      value: 'unconfirmed-token',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const loaded = submittedSettingsEntry(entry, {
      environment: { set: { NEW_TOKEN: 'unconfirmed-token' }, unset: [] },
    })
    const rebased = rebaseSettingsDraft(entry, pending, loaded)
    expect(rebased.env.at(-1)).toMatchObject({
      key: 'NEW_TOKEN',
      value: 'unconfirmed-token',
      isNew: false,
      secret: true,
      replacing: true,
    })
    expect(settingsPatch(loaded, rebased).patch.environment).toEqual({
      set: { NEW_TOKEN: 'unconfirmed-token' },
      unset: [],
    })
  })

  it('does not repeat a removal already reflected in readback and retains edited missing values', () => {
    const baseline = submittedSettingsEntry(entry, {
      environment: { set: { EDITED: 'written' }, unset: [] },
    })
    const pending = draftFrom(baseline)
    pending.env[0].removed = true
    pending.env[2].value = 'newer'
    const loaded = { ...baseline, environment: [baseline.environment[1]] }
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    expect(rebased.env.map((row) => row.key)).toEqual([
      'OPENAI_API_KEY',
      'EDITED',
    ])
    expect(settingsPatch(loaded, rebased).patch.environment).toEqual({
      set: { EDITED: 'newer' },
      unset: [],
    })
  })

  it('does not mutate either snapshot or the retained draft', () => {
    const baseline = submittedSettingsEntry(entry, { run: 'written-command' })
    const pending = draftFrom(baseline)
    pending.startAfter = ['state']
    pending.env[0].value = 'debug'
    const loaded = structuredClone(baseline)
    const before = structuredClone({ baseline, pending, loaded })
    const rebased = rebaseSettingsDraft(baseline, pending, loaded)
    rebased.startAfter.push('later')
    rebased.env[0].value = 'trace'
    expect({ baseline, pending, loaded }).toEqual(before)
  })
})

describe('confirmStoredPatch', () => {
  it('confirms normalized commands and dependencies independent of their order', () => {
    expect(
      confirmStoredPatch(
        { ...entry, run: 'cargo run ', start_after: ['storage', 'state'] },
        { run: 'cargo run', start_after: ['state', 'storage'] },
      ),
    ).toBe(true)
  })

  it.each<EditPatch>([
    { run: 'another command' },
    { start_after: ['missing'] },
    { config_override: 'data_dir: /another/path' },
    { environment: { set: { RUST_LOG: 'debug' }, unset: [] } },
    { environment: { set: { MISSING: '' }, unset: [] } },
    { environment: { set: {}, unset: ['RUST_LOG'] } },
  ])('rejects a readback that differs from an observable patch %j', (patch) => {
    expect(confirmStoredPatch(entry, patch)).toBe(false)
  })

  it('compares environment sets and unsets by key, including empty values', () => {
    expect(
      confirmStoredPatch(
        {
          ...entry,
          environment: [
            { key: 'EMPTY', value: null, secret: false },
            { key: 'RUST_LOG', value: 'debug', secret: false },
          ],
        },
        {
          environment: {
            set: { RUST_LOG: 'debug', EMPTY: '' },
            unset: ['OPENAI_API_KEY'],
          },
        },
      ),
    ).toBe(true)
  })

  it('confirms a replacement that sets a key after unsetting its old row', () => {
    expect(
      confirmStoredPatch(entry, {
        environment: { set: { RUST_LOG: 'info' }, unset: ['RUST_LOG'] },
      }),
    ).toBe(true)
  })

  it('returns unknown when a patched secret is masked in an otherwise matching readback', () => {
    expect(
      confirmStoredPatch(entry, {
        run: entry.run ?? '',
        environment: { set: { OPENAI_API_KEY: 'new-secret' }, unset: [] },
      }),
    ).toBeNull()
  })

  it('returns false for a mismatch even when another patched field is masked', () => {
    expect(
      confirmStoredPatch(entry, {
        run: 'another command',
        environment: { set: { OPENAI_API_KEY: 'new-secret' }, unset: [] },
      }),
    ).toBe(false)
  })

  it('compares configuration values semantically through serialization changes', () => {
    expect(
      confirmStoredPatch(
        {
          ...entry,
          config_override: 'http:\n  host: localhost\n  port: 3000\n',
        },
        { config_override: 'http: {port: 3000, host: localhost}' },
      ),
    ).toBe(true)
  })

  it('matches empty configuration to a cleared override', () => {
    expect(
      confirmStoredPatch(
        { ...entry, config_override: null },
        { config_override: '' },
      ),
    ).toBe(true)
  })

  it('keeps scalar types and array order significant in configuration', () => {
    expect(
      confirmStoredPatch(
        { ...entry, config_override: 'ports: [3000, 4000]\nenabled: true\n' },
        { config_override: 'ports: [4000, 3000]\nenabled: "true"\n' },
      ),
    ).toBe(false)
  })
})

describe('isSecretValue', () => {
  it.each([
    'api_key',
    'ACCESS_TOKEN',
    'SECRET',
    'password',
    'credentials',
    'MONKEY',
  ])('classifies literal %j using the backend credential rule', (key) => {
    expect(isSecretValue(key, 'literal')).toBe(true)
  })

  it.each(['', `\${API_KEY}`])(
    'keeps empty or referenced credentials %j visible',
    (value) => {
      expect(isSecretValue('API_KEY', value)).toBe(false)
    },
  )

  it('does not mask ordinary values or treat partial references as templates', () => {
    expect(isSecretValue('PORT', '3000')).toBe(false)
    expect(isSecretValue('API_KEY', '${API_KEY')).toBe(true)
  })
})

describe('entryYaml', () => {
  it('renders the block and keeps secrets masked, even a replaced one', () => {
    const draft = draftFrom(entry)
    draft.env[1] = { ...draft.env[1], replacing: true, value: 'sk-new' }
    const yaml = entryYaml(entryShape(entry, draft))
    expect(yaml).toContain(`OPENAI_API_KEY: ${MASK}  # new value`)
    expect(yaml).not.toContain('sk-new')
    expect(entryYaml(entryShape(entry))).toBe(
      [
        '  harness-e2e:',
        '    worker: path:///home/me/wt/harness-e2e',
        '    env_file: [.env]',
        '    config_override:',
        '      data_dir: ~/.iii/data/harness-e2e',
        '    environment:',
        '      RUST_LOG: info',
        `      OPENAI_API_KEY: ${MASK}`,
        '    scripts:',
        '      run: ./target/debug/harness-e2e',
        '',
      ].join('\n'),
    )
  })

  it('quotes values YAML would read as something else', () => {
    const yaml = entryYaml({
      name: 'x',
      worker: 'package://r/x',
      version: '1.0',
      env: [{ key: 'PORT', value: '3113' }],
    })
    expect(yaml).toContain('version: "1.0"')
    expect(yaml).toContain('PORT: "3113"')
  })

  it('masks a credential added before the backend has classified its row', () => {
    const draft = draftFrom(entry)
    draft.env.push({
      key: 'ACCESS_TOKEN',
      value: 'typed-secret',
      secret: false,
      replacing: false,
      removed: false,
      isNew: true,
    })
    const yaml = entryYaml(entryShape(entry, draft))
    expect(yaml).toContain(`ACCESS_TOKEN: ${MASK}`)
    expect(yaml).not.toContain('typed-secret')
  })

  it('masks a literal replacing an initially visible credential reference', () => {
    const referenced: ContainerEntry = {
      ...entry,
      environment: [{ key: 'API_KEY', value: `\${API_KEY}`, secret: false }],
    }
    expect(entryYaml(entryShape(referenced))).toContain(`\${API_KEY}`)
    const draft = draftFrom(referenced)
    draft.env[0].value = 'new-literal'
    const yaml = entryYaml(entryShape(referenced, draft))
    expect(yaml).toContain(`API_KEY: ${MASK}`)
    expect(yaml).not.toContain('new-literal')
  })
})

describe('parseLogLine', () => {
  it('splits a tracing line into local time, level, target and text', () => {
    const parts = parseLogLine(
      '2026-10-01T00:22:53.685636Z  INFO llm_router::triggers: trigger subscription registered',
    )
    const local = new Date('2026-10-01T00:22:53.685Z')
    expect(parts).toMatchObject({
      level: 'INFO',
      target: 'llm_router::triggers',
      text: 'trigger subscription registered',
      stamp: '2026-10-01T00:22:53.685636Z',
    })
    expect(parts?.time).toBe(
      `${String(local.getHours()).padStart(2, '0')}:${String(local.getMinutes()).padStart(2, '0')}:53.685`,
    )
  })

  it('leaves anything else raw', () => {
    expect(parseLogLine('compiling queue v0.1.0')).toBeNull()
    expect(
      parseLogLine('\u001b[2m2026-10-01T00:22:53Z\u001b[0m INFO a: b'),
    ).toBeNull()
  })
})

describe('waveLabel', () => {
  const decl = (name: string, start_after: string[] = []) =>
    ({ name, start_after }) as unknown as DeclaredContainer
  const all = [
    decl('state'),
    decl('router', ['state']),
    decl('canvas', ['state']),
    decl('harness', ['router', 'state', 'gone']),
  ]

  it('names a single dependency and counts several', () => {
    expect(waveLabel(['router', 'canvas'], all)).toBe('After state')
    expect(waveLabel(['harness'], all)).toBe('After 2 containers')
  })
})

describe('arrangeCheckouts', () => {
  const at = (
    path: string,
    committed_at: number,
    branch: string | null = null,
  ) => ({ path, branch, committed_at })
  const all = [
    at('/w/old/web', 10, 'feat/old'),
    at('/w/main/web', 30, 'main'),
    at('/w/web-copy', 50, 'feat/copy'),
    at('/w/run/web', 20, null),
  ]

  it('puts the current checkout first, then the newest, and the other folders apart', () => {
    const { usable, other } = arrangeCheckouts(all, 'web', '/w/run/web')
    expect(usable.map((c) => c.path)).toEqual([
      '/w/run/web',
      '/w/main/web',
      '/w/old/web',
    ])
    expect(other.map((c) => c.path)).toEqual(['/w/web-copy'])
  })

  it('filters by branch or folder, detached included', () => {
    expect(arrangeCheckouts(all, 'web', '', 'OLD').usable).toHaveLength(1)
    expect(arrangeCheckouts(all, 'web', '', 'detached').usable[0]?.path).toBe(
      '/w/run/web',
    )
    expect(arrangeCheckouts(all, 'web', '', 'copy').other).toHaveLength(1)
  })
})

describe('matchParts', () => {
  it('splits around the first case-insensitive match', () => {
    expect(matchParts('feat/Trends-api', 'trends')).toEqual([
      'feat/',
      'Trends',
      '-api',
    ])
    expect(matchParts('main', 'x')).toEqual(['main', '', ''])
    expect(matchParts('main', ' ')).toEqual(['main', '', ''])
  })
})

describe('inPlaceVersion', () => {
  it('changes a version in place only for a container named after its package', () => {
    const pkg = (name: string, ref: string) =>
      ({ name, ref, source: 'package' }) as const
    expect(inPlaceVersion(pkg('storage', 'api.workers.iii.dev/storage'))).toBe(
      true,
    )
    expect(inPlaceVersion(pkg('db', 'api.workers.iii.dev/database'))).toBe(
      false,
    )
    expect(
      inPlaceVersion({ name: 'queue', ref: '/w/queue', source: 'path' }),
    ).toBe(false)
  })
})

describe('alsoStarts', () => {
  it('names the stopped containers an add would bring up, minus the target', () => {
    expect(alsoStarts(['harness-e2e', 'web'], ['web'])).toEqual([
      'Also starts harness-e2e, stopped now',
    ])
    expect(alsoStarts(['web'], ['web'])).toEqual([])
  })
})

describe('defaultRun', () => {
  it('offers cargo run of the binary a Rust worker builds, nothing for the rest', () => {
    expect(defaultRun({ name: 'judge', language: 'rust', bin: null })).toBe(
      'cargo run --locked --bin judge',
    )
    expect(
      defaultRun({ name: 'judge', language: 'rust', bin: 'judge-typesafe' }),
    ).toBe('cargo run --locked --bin judge-typesafe')
    expect(defaultRun({ name: 'web', language: 'node', bin: null })).toBe('')
  })
})
