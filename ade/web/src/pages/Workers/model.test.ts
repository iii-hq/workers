import { describe, expect, it } from 'vitest'
import type { ContainerEntry, DeclaredContainer } from './compose-api'
import {
  alsoStarts,
  arrangeCheckouts,
  defaultRun,
  dependentsOf,
  draftFrom,
  entryShape,
  entryYaml,
  groupContainers,
  inPlaceVersion,
  MASK,
  matchParts,
  parseLogLine,
  settingsPatch,
  shortPath,
  startWaves,
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
      state: 'workers',
      'harness-e2e': 'scenario-editorial',
    })
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
    expect(settingsPatch(entry, draft).changes).toBe(0)
    draft.env[1].value = 'sk-new'
    draft.env[0].removed = true
    expect(settingsPatch(entry, draft).patch.environment).toEqual({
      set: { OPENAI_API_KEY: 'sk-new' },
      unset: ['RUST_LOG'],
    })
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
