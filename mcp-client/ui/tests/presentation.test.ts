import { describe, expect, it } from 'vitest'
import { gateStatus, type Verdict } from '../src/gate-rules'
import {
  accessAction,
  accessConfirm,
  accessLists,
  advertisesText,
  announcements,
  blocksOf,
  byName,
  connectionRowText,
  countdown,
  countLabel,
  drawsWhole,
  elapsedText,
  entryError,
  filterRows,
  firstVariable,
  functionRows,
  highlightable,
  holdSnapshot,
  inputsText,
  jsonText,
  matchesQuery,
  messagesOf,
  middleTruncate,
  narrowLevel,
  pageState,
  pathHeads,
  presentServer,
  processFacts,
  processRowText,
  readOnlyEndpoint,
  resourceMeta,
  resultSummary,
  reviewAccessPanel,
  rowGroups,
  runConfirm,
  STAGES,
  STATE_WORD,
  safeAuthorizeLink,
  searchNote,
  signInControls,
  signInStatus,
  signInView,
  skippedText,
  stageTrack,
  statusBarText,
  uniqueBy,
} from '../src/presentation'
import type { EngineFunction, Item, LoginStart, Server, Skipped } from '../src/types'
import type { ServersData, Snapshot } from '../src/use-servers'
import { fa, L, rule } from './fixtures/gate'
import { docs, list, NOW, server, server as srv, stdio } from './fixtures/servers'

const err = (code: string, message: string) => ({ code, message }) as never
const exited = (code: number | null, signal: number | null, next: number | null) =>
  stdio({
    state: 'unavailable',
    error: err('exited', 'exited with code 1: ENOENT'),
    process: {
      pid: null,
      started_at: null,
      restarts: 2,
      restart_budget: 5,
      next_restart_at: next,
      last_exit: { code, signal },
    },
  })

describe('presentServer', () => {
  it.each<[Server, string, boolean, string, { id: string; label: string } | null]>([
    [server(), 'ok', false, 'mcp.linear.app · API key', null],
    [server({ auth: 'none' }), 'ok', false, 'mcp.linear.app', null],
    [server({ auth: 'client_credentials' }), 'ok', false, 'mcp.linear.app · Client credentials', null],
    [server({ auth: 'oauth' }), 'ok', false, 'mcp.linear.app · Sign-in', null],
    [server({ target: 'secret://LINEAR_URL' }), 'ok', false, 'secret://LINEAR_URL · API key', null],
    [stdio(), 'ok', false, 'stdio · pid 48213', null],
    [server({ state: 'connecting', stage: 'secret' }), 'accent', true, 'Connecting… · Secret', null],
    [server({ state: 'needs_login' }), 'warn', false, 'Needs sign-in', { id: 'sign_in', label: 'Sign in' }],
    [
      server({ state: 'auth_failed', error: err('credential_rejected', 'x') }),
      'alert',
      false,
      'Auth failed · 401',
      { id: 'update_key', label: 'Update key' },
    ],
    [
      server({
        state: 'auth_failed',
        auth: 'client_credentials',
        error: err('credential_rejected', 'invalid_client'),
      }),
      'alert',
      false,
      'Auth failed',
      { id: 'update_key', label: 'Update key' },
    ],
    [
      server({ state: 'config_error', error: err('secret_not_shared', 'x') }),
      'alert',
      false,
      'Config error · secret not shared',
      { id: 'share', label: 'Share' },
    ],
    [
      server({ state: 'config_error', error: err('invalid_config', 'url is not a valid URL') }),
      'alert',
      false,
      'Config error · url is not a valid URL',
      { id: 'fix', label: 'Fix' },
    ],
    [
      server({ state: 'config_error', error: err('invalid_config', '/mcpServers/linear: set `url` or `command`, not both') }),
      'alert',
      false,
      'Config error · set `url` or `command`, not both',
      { id: 'fix', label: 'Fix' },
    ],
    [
      server({ state: 'unavailable', error: err('connect_failed', 'connection refused') }),
      'alert',
      false,
      'Unavailable · retrying',
      { id: 'retry', label: 'Retry' },
    ],
    [exited(1, null, NOW + 7_200), 'alert', false, 'Exited 1 · restarting in 8 s', { id: 'restart', label: 'Restart' }],
    [
      exited(null, 9, NOW + 7_200),
      'alert',
      false,
      'Killed by signal 9 · restarting in 8 s',
      { id: 'restart', label: 'Restart' },
    ],
    [exited(1, null, null), 'alert', false, 'Exited 1', { id: 'restart', label: 'Restart' }],
    [
      server({ state: 'unapproved' }),
      'warn',
      false,
      'Review needed · changed outside this page',
      { id: 'review', label: 'Review' },
    ],
    [server({ state: 'disabled' }), 'ghost', false, 'Off', { id: 'turn_on', label: 'Turn on' }],
  ])('%#', (s, dot, pulse, description, action) => {
    expect(presentServer(s, NOW)).toMatchObject({ dot, pulse, description, action })
  })

  it('keeps the worker text apart from the next step', () => {
    expect(presentServer(server(), NOW).panel).toBeNull()
    expect(presentServer(server({ state: 'disabled' }), NOW).panel).toBeNull()
    expect(presentServer(server({ state: 'needs_login' }), NOW).panel).toMatchObject({
      variant: 'warn',
      headline: 'Sign in to connect',
    })
    expect(presentServer(server({ state: 'unapproved' }), NOW).panel).toMatchObject({
      variant: 'warn',
      headline: 'Review needed',
    })
    expect(
      presentServer(server({ state: 'unavailable', error: err('connect_failed', 'connection refused') }), NOW).panel,
    ).toEqual({ variant: 'alert', headline: 'connection refused', next: 'The worker keeps trying by itself.' })
    expect(presentServer(exited(1, null, NOW + 1), NOW).panel?.headline).toBe('exited with code 1: ENOENT')
  })
})

describe('countLabel', () => {
  it('labels a server by what it holds', () => {
    expect(countLabel(server())).toBe('2 tools')
    expect(countLabel(server({ tools: server().tools.slice(0, 1) }))).toBe('1 tool')
    expect(countLabel(server({ tools: [] }))).toBe('0 tools')
    // Linear: tools and instructions, nothing else
    expect(countLabel(server({ has_instructions: true }))).toBe('2 tools')
    // 1 tool, instructions, 3 resource functions, 1 prompt
    expect(countLabel(docs())).toBe('6 functions')
  })
})

describe('statusBarText', () => {
  it('writes the status bar', () => {
    const broken = (['needs_login', 'auth_failed', 'unavailable', 'unapproved'] as const).map((state) =>
      server({ name: state, state, tools: [] }),
    )
    expect(statusBarText(list([server(), server({ name: 'b' }), ...broken]))).toBe(
      '6 servers · 2 ready · 4 functions registered · 4 need attention',
    )
    expect(statusBarText(list([server({ tools: server().tools.slice(0, 1) })]))).toBe(
      '1 server · 1 ready · 1 function registered · 0 need attention',
    )
    expect(statusBarText(list([server({ state: 'needs_login', tools: [] })]))).toBe(
      '1 server · 0 ready · 0 functions registered · 1 needs attention',
    )
    expect(
      statusBarText(
        list([server({ state: 'disabled' }), server({ name: 'b', state: 'connecting', stage: 'connect' })]),
      ),
    ).toContain('0 need attention')
  })
})

describe('STAGES', () => {
  it('names the six stages in order', () => {
    expect(STAGES.map((s) => s.label)).toEqual(['Config', 'Approval', 'Secret', 'Connect', 'Authorize', 'Tools'])
  })
})

describe('pathHeads', () => {
  const fn = (functionId: string): Item => ({ kind: 'function', functionId })

  it('spells the function id across the three heads', () => {
    expect(pathHeads({ server: null, item: null }, null, 6)).toEqual({
      first: 'mcp::',
      firstMeta: '6 servers',
      second: null,
      secondMeta: null,
      third: null,
      thirdIsId: false,
    })
    expect(pathHeads({ server: 'linear', item: fn('mcp::linear::list_issues') }, server(), 6)).toEqual({
      first: 'mcp::',
      firstMeta: '6 servers',
      second: 'linear::',
      secondMeta: '2 tools',
      third: 'list_issues',
      thirdIsId: true,
    })
    expect(pathHeads({ server: 'docs', item: fn('mcp::docs::resources::read') }, docs(), 1)).toMatchObject({
      firstMeta: '1 server',
      secondMeta: '6 functions',
      third: 'resources::read',
      thirdIsId: true,
    })
    expect(pathHeads({ server: 'docs', item: fn('mcp::docs::prompts::code_review') }, docs(), 1).third).toBe(
      'prompts::code_review',
    )
  })

  it('shows a word, not an id, for the pinned rows', () => {
    for (const [kind, third] of [
      ['connection', 'Connection'],
      ['process', 'Process'],
      ['access', 'Agent access'],
    ] as const) {
      expect(pathHeads({ server: 'linear', item: { kind } }, server(), 1)).toMatchObject({ third, thirdIsId: false })
    }
  })

  it('reads 0 servers on first run and keeps a name the worker did not report', () => {
    expect(pathHeads({ server: null, item: null }, null, 0).firstMeta).toBe('0 servers')
    expect(pathHeads({ server: 'linear', item: null }, null, 1)).toMatchObject({
      second: 'linear::',
      secondMeta: null,
      third: null,
    })
  })
})

describe('pageState', () => {
  const data = (over: Partial<ServersData> = {}): ServersData => ({
    list: list([server()]),
    config: { linear: {} },
    functions: {},
    workerDown: false,
    configDown: false,
    configError: null,
    stale: false,
    loading: false,
    error: null,
    refresh: () => {},
    ...over,
  })

  it('tells the five situations apart by which read failed', () => {
    expect(pageState(data({ loading: true, list: null, config: null }))).toBe('loading')
    // a re-read never flashes the skeleton
    expect(pageState(data({ loading: true }))).toBe('ready')
    expect(pageState(data({ configDown: true, config: null }))).toBe('config_down')
    expect(pageState(data({ configDown: true, workerDown: true, list: null, config: null }))).toBe('config_down')
    expect(pageState(data({ workerDown: true, list: null }))).toBe('worker_down')
    expect(pageState(data({ list: list([]), config: {} }))).toBe('first_run')
    expect(pageState(data({}))).toBe('ready')
  })
})

describe('holdSnapshot', () => {
  const fn = { function_id: 'mcp::linear::whoami' } as EngineFunction
  const good: Snapshot = {
    list: list([server()]),
    config: { linear: {} },
    functions: { linear: [fn] },
    workerDown: false,
    configDown: false,
    functionsDown: false,
    configError: null,
    stale: false,
  }
  const failed: Snapshot = {
    list: null,
    config: null,
    functions: {},
    workerDown: true,
    configDown: true,
    functionsDown: true,
    configError: 'timeout',
    stale: false,
  }
  const rest = { loading: false, error: null, refresh: () => {} }

  it('keeps the last good read of each part that failed, and says it is stale', () => {
    const held = holdSnapshot(good, failed)
    expect(held).toMatchObject({ list: good.list, config: good.config, functions: good.functions, stale: true })
    expect([held.workerDown, held.configDown, held.functionsDown]).toEqual([false, false, false])
    expect(pageState({ ...held, ...rest })).toBe('ready')
    // A second failure keeps holding the same read.
    expect(holdSnapshot(held, failed)).toEqual(held)
  })

  it('passes a failure through when nothing is held, and a good read as it is', () => {
    const first = holdSnapshot(null, failed)
    expect(first).toEqual(failed)
    expect(pageState({ ...first, ...rest })).toBe('config_down')
    const good2 = { ...good, config: { linear: {}, docs: {} } }
    expect(holdSnapshot(good, good2)).toEqual(good2)
    expect(holdSnapshot(holdSnapshot(good, failed), good2).stale).toBe(false)
  })
})

describe('byName', () => {
  it('sorts by code unit, the same in every locale', () => {
    expect(['linear_x', 'linear-me', 'Zeta', 'alpha'].sort(byName)).toEqual(['Zeta', 'alpha', 'linear-me', 'linear_x'])
    expect([byName('a', 'b'), byName('b', 'a'), byName('a', 'a')]).toEqual([-1, 1, 0])
  })
})

describe('column 2', () => {
  const fns: EngineFunction[] = [
    { function_id: 'mcp::linear::list_issues', description: 'List issues in the workspace' },
  ]
  const tool = (name: string): Skipped => ({ kind: 'tool', name, reason: 'name has a character outside [A-Za-z0-9_-]' })

  it('builds rows in three groups, tools first', () => {
    expect(functionRows(server(), fns)).toEqual([
      {
        functionId: 'mcp::linear::list_issues',
        group: 'tools',
        name: 'list_issues',
        description: 'List issues in the workspace',
        chip: null,
      },
      { functionId: 'mcp::linear::save_issue', group: 'tools', name: 'save_issue', description: '', chip: 'Writes' },
    ])
    expect(functionRows(docs(), []).map((r) => [r.group, r.name, r.chip])).toEqual([
      ['tools', 'instructions', null],
      ['tools', 'search', 'Destructive'],
      ['resources', 'resources::list', null],
      ['resources', 'resources::read', null],
      ['resources', 'resources::templates', null],
      ['prompts', 'code_review', null],
    ])
  })

  it('says when the engine does not hold a function', () => {
    const [first, second] = server().tools
    const s = server({ tools: [{ ...first, registered: false }, second] })
    expect(functionRows(s, fns)[0].description).toBe('Not registered in the engine')
  })

  it('filters all groups and counts the matches', () => {
    const rows = functionRows(server(), fns)
    expect(filterRows(rows, '')).toEqual({ rows, countText: '2' })
    expect(filterRows(rows, '  SAVE ')).toEqual({ rows: [rows[1]], countText: '1 of 2' })
    // the description matches too
    expect(filterRows(rows, 'workspace').rows).toEqual([rows[0]])
    expect(filterRows(rows, 'zzz')).toEqual({ rows: [], countText: '0 of 2' })
  })

  it('counts skipped names', () => {
    expect(skippedText(server())).toBeNull()
    expect(skippedText(server({ skipped: [tool('a.b')] }))).toBe('1 tool skipped')
    expect(skippedText(server({ skipped: [tool('a.b'), tool('c d'), tool('e/f')] }))).toBe('3 tools skipped')
    expect(
      skippedText(server({ skipped: [tool('a.b'), tool('c d'), { kind: 'prompt', name: 'bad.name', reason: 'x' }] })),
    ).toBe('3 skipped')
  })

  it('truncates in the middle', () => {
    expect(middleTruncate('abcdefghij', 7)).toBe('abc…hij')
    expect(middleTruncate('abcdefghij', 6)).toBe('abc…ij')
    expect(middleTruncate('abc', 7)).toBe('abc')
  })

  it('words the pinned rows', () => {
    expect(STATE_WORD).toEqual({
      ready: 'Ready',
      connecting: 'Connecting',
      needs_login: 'Needs sign-in',
      auth_failed: 'Auth failed',
      config_error: 'Config error',
      unavailable: 'Unavailable',
      unapproved: 'Review needed',
      disabled: 'Off',
    })
    expect(connectionRowText(server())).toBe('HTTP · API key · protocol 2025-11-25')
    expect(connectionRowText(stdio({ protocol_version: null }))).toBe('stdio')
    expect(connectionRowText(server({ auth: 'oauth', state: 'needs_login', protocol_version: null }))).toBe('HTTP · Sign-in')
    const proc = stdio().process
    expect(processRowText(proc, NOW)).toBe('running · pid 48213 · up 2h 14m')
    expect(processRowText(proc && { ...proc, pid: null, started_at: null }, NOW)).toBe('not running')
    expect(processRowText(null, NOW)).toBe('not running')
  })

  it('announces a server that becomes ready, once', () => {
    expect(announcements(list([server({ state: 'connecting' })]), list([server()]))).toEqual([
      'linear connected, 2 tools',
    ])
    expect(announcements(null, list([server()]))).toEqual([])
    expect(announcements(list([server()]), list([server()]))).toEqual([])
  })

  it('groups the rows: Tools always, the others with their capability, only matches while filtering', () => {
    const shape = (s: Server, query = '', rows = filterRows(functionRows(s, []), query).rows) =>
      rowGroups(s, rows, query).map((g) => [g.label, g.count, g.rows.length, g.empty, g.failed?.headline ?? null])
    // the instructions row is listed under Tools but not counted as a tool
    expect(shape(docs())).toEqual([
      ['Tools', 1, 2, null, null],
      ['Resources', 3, 3, null, null],
      ['Prompts', 1, 1, null, null],
    ])
    expect(shape(server({ tools: [], tools_listed: false }))).toEqual([
      ['Tools', 0, 0, 'No tools. Not connected yet.', null],
    ])
    expect(shape(server({ tools: [] }))).toEqual([['Tools', 0, 0, 'No tools. The server lists none.', null]])
    const prompts = { resources: false, prompts: true }
    expect(shape(server({ capabilities: prompts }))).toEqual([
      ['Tools', 2, 2, null, null],
      ['Prompts', 0, 0, 'No prompts', null],
    ])
    const failed = server({ capabilities: prompts, list_errors: { prompts: 'timed out', templates: 'boom' } })
    expect(shape(failed)).toEqual([
      ['Tools', 2, 2, null, null],
      ['Resources', 0, 0, null, 'Could not list resource templates'],
      ['Prompts', 0, 0, null, 'Could not list prompts'],
    ])
    expect(rowGroups(failed, [], '')[2].failed).toEqual({ headline: 'Could not list prompts', detail: 'timed out' })
    expect(shape(docs(), ' code ')).toEqual([['Prompts', 1, 1, null, null]])
  })

  it('shows a page-level state in place of the server list when narrow', () => {
    expect(narrowLevel('ready', 'detail')).toBe('detail')
    expect(narrowLevel('loading', 'servers')).toBe('servers')
    expect(narrowLevel('first_run', 'servers')).toBe('contents')
    expect(narrowLevel('worker_down', 'servers')).toBe('contents')
    expect(narrowLevel('config_down', 'servers')).toBe('contents')
  })
})

describe('inputsText', () => {
  const schema = (n: number, required: string[] = []) => ({
    type: 'object',
    properties: Object.fromEntries(Array.from({ length: n }, (_, i) => [`p${i}`, { type: 'string' }])),
    required,
  })

  it('counts inputs for tools and resource functions', () => {
    expect(inputsText(schema(12), 'tools')).toBe('12 inputs, none required')
    expect(inputsText(schema(3, ['p0']), 'tools')).toBe('3 inputs, 1 required')
    expect(inputsText(schema(1, ['p0']), 'resources')).toBe('1 input, 1 required')
    expect(inputsText(schema(0), 'tools')).toBe('no inputs')
  })

  it('says arguments for a prompt', () => {
    expect(inputsText(schema(2, ['p0']), 'prompts')).toBe('2 arguments, 1 required')
    expect(inputsText(schema(0), 'prompts')).toBe('no arguments')
  })

  it('is nothing until the schema is read', () => {
    expect(inputsText(undefined, 'tools')).toBeNull()
  })

  it('is nothing when a union hides the inputs, and counts none for no schema', () => {
    const union = { anyOf: [schema(1, ['p0']), schema(2)] }
    expect(inputsText(union, 'tools')).toBeNull()
    expect(inputsText(null, 'tools')).toBe('no inputs')
  })
})

describe('test results', () => {
  const text = (n: number) => ({ type: 'text', text: 'x'.repeat(n) })

  it('summarises a tool result', () => {
    expect(resultSummary({ content: [text(2355)] }, 412)).toBe('OK · 412 ms · 1 text block · 2.3 KiB')
    expect(
      resultSummary(
        { content: [text(10), text(10), { type: 'image', mime: 'image/png', data: 'A'.repeat(1368) }] },
        1234,
      ),
    ).toBe('OK · 1.2 s · 2 text blocks · 1 image · 1.0 KiB')
    expect(resultSummary({ content: [] }, 88)).toBe('OK · 88 ms · empty')
    expect(resultSummary({ content: [{ type: 'text', text: '日本' }] }, 5)).toBe('OK · 5 ms · 1 text block · 6 B')
  })

  it('summarises a prompt result and a list page', () => {
    expect(resultSummary({ messages: [{}, {}, {}] }, 120)).toBe('OK · 120 ms · 3 messages')
    expect(resultSummary({ messages: [{}] }, 120)).toBe('OK · 120 ms · 1 message')
    // The bytes of {"resources":[]}.
    expect(resultSummary({ resources: [] }, 88)).toBe('OK · 88 ms · 16 B')
  })

  it('formats elapsed time', () => {
    expect([0, 999, 1000, 1249, 61_000].map(elapsedText)).toEqual(['0 ms', '999 ms', '1.0 s', '1.2 s', '61.0 s'])
  })

  it('keeps only blocks it can draw', () => {
    const image = { type: 'image', mime: 'image/png', data: 'AA' }
    expect(blocksOf({ content: [text(1), image, { type: 'audio' }, null] })).toEqual([text(1), image])
    expect(blocksOf({ resources: [] })).toEqual([])
    expect(blocksOf(null)).toEqual([])
  })

  it('highlights text that parses to an object or an array, up to 32,768 characters', () => {
    expect(jsonText('{"a":[1]}')).toBe('{\n  "a": [\n    1\n  ]\n}')
    expect(jsonText('[]')).toBe('[]')
    for (const text of ['42', '"x"', 'null', 'not json', '']) expect(jsonText(text)).toBeNull()
    expect(jsonText(`[${'1,'.repeat(16_384)}1]`)).toBeNull()
    expect([highlightable('x'.repeat(32_768)), highlightable('x'.repeat(32_769))]).toEqual([true, false])
  })

  it('draws a result whole when it has no block and no messages', () => {
    expect(drawsWhole({ resources: [] })).toBe(true)
    expect(drawsWhole({ content: [] })).toBe(true)
    expect(drawsWhole({ content: [text(1)] })).toBe(false)
    expect(drawsWhole({ messages: [] })).toBe(false)
  })

  it('asks before a write and never before a read', () => {
    expect(runConfirm('linear', 'list_issues', 'read')).toBeNull()
    expect(runConfirm('linear', 'save_issue', 'write')).toEqual({
      title: 'Run `save_issue` on linear? This tool can change data.',
      tone: 'default',
    })
    expect(runConfirm('linear', 'delete_comment', 'destructive')).toEqual({
      title: 'Run `delete_comment` on linear? This tool is destructive and can change data.',
      tone: 'danger',
    })
  })
})

describe('resource read and prompt run', () => {
  it('finds the first template variable', () => {
    expect(firstVariable('file:///{name}.txt')).toEqual({ start: 8, end: 14 })
    expect(firstVariable('repo://{owner}/{repo}')).toEqual({ start: 7, end: 14 })
    expect(firstVariable('file:///README.md')).toBeNull()
    expect(firstVariable('file:///{}')).toBeNull()
  })

  it('says when the search covers only what is loaded', () => {
    expect(searchNote(200, true)).toBe('Searching 200 loaded resources')
    expect(searchNote(1, true)).toBe('Searching 1 loaded resource')
    expect(searchNote(200, false)).toBeNull()
  })

  it('reads prompt messages with authored role labels', () => {
    expect(
      messagesOf({
        description: 'Review code',
        messages: [
          { role: 'user', content: [{ type: 'text', text: 'Review this' }] },
          { role: 'assistant', content: [{ type: 'image', mime: 'image/png', data: 'AA' }, { type: 'audio' }] },
        ],
      }),
    ).toEqual({
      description: 'Review code',
      messages: [
        { role: 'User', blocks: [{ type: 'text', text: 'Review this' }] },
        { role: 'Assistant', blocks: [{ type: 'image', mime: 'image/png', data: 'AA' }] },
      ],
    })
    expect(messagesOf({ messages: [] })).toEqual({ description: null, messages: [] })
    expect(messagesOf({ content: [] })).toEqual({ description: null, messages: [] })
  })

  it('filters picker rows by any of their texts and reads their meta', () => {
    expect(matchesQuery(' README ', 'readme', 'file:///a.txt')).toBe(true)
    expect(matchesQuery('a.txt', 'readme', 'file:///a.txt')).toBe(true)
    expect(matchesQuery('b.txt', 'readme', 'file:///a.txt')).toBe(false)
    expect(matchesQuery('', 'readme')).toBe(true)
    expect(resourceMeta('text/plain', 2048)).toBe('text/plain · 2.0 KiB')
    expect(resourceMeta(undefined, 5)).toBe('5 B')
    expect(resourceMeta(undefined, undefined)).toBe('')
  })

  it('keeps one row per uri across pages, the first in place', () => {
    const row = (uri: string, name = uri) => ({ uri, name })
    const loaded = [row('file:///a'), row('file:///b')]
    const page = [row('file:///b', 'b again'), row('file:///c')]
    expect(uniqueBy([...loaded, ...page], (r) => r.uri)).toEqual([row('file:///a'), row('file:///b'), row('file:///c')])
    const template = (uri_template: string, name: string) => ({ uri_template, name })
    expect(
      uniqueBy(
        [
          template('file:///{name}.txt', 'Text file'),
          template('repo://{owner}', 'Repo'),
          template('file:///{name}.txt', 'Again'),
        ],
        (t) => t.uri_template,
      ),
    ).toEqual([template('file:///{name}.txt', 'Text file'), template('repo://{owner}', 'Repo')])
  })
})

describe('stageTrack', () => {
  const tones = (s: Server | null) => stageTrack(s).map((stage) => stage.tone)
  const ghosts = Array(6).fill('ghost')

  it('shows six labelled stages, none reached for a draft', () => {
    expect(stageTrack(null).map((s) => s.label)).toEqual([
      'Config',
      'Approval',
      'Secret',
      'Connect',
      'Authorize',
      'Tools',
    ])
    expect(tones(null)).toEqual(ghosts)
  })

  it('marks how far the connection got and where it stopped', () => {
    expect(tones(srv({ state: 'ready', stage: null }))).toEqual(Array(6).fill('ok'))
    const connecting = srv({ state: 'connecting', stage: 'connect' })
    expect(stageTrack(connecting)[3]).toEqual({ id: 'connect', label: 'Connect', tone: 'accent', pulse: true })
    expect(tones(connecting)).toEqual(['ok', 'ok', 'ok', 'accent', 'ghost', 'ghost'])
    expect(tones(srv({ state: 'unapproved', stage: 'approval' }))).toEqual([
      'ok',
      'warn',
      'ghost',
      'ghost',
      'ghost',
      'ghost',
    ])
    expect(tones(srv({ state: 'needs_login', stage: 'authorize' }))).toEqual(['ok', 'ok', 'ok', 'ok', 'warn', 'ghost'])
    expect(tones(srv({ state: 'config_error', stage: 'config' }))).toEqual([
      'alert',
      'ghost',
      'ghost',
      'ghost',
      'ghost',
      'ghost',
    ])
    expect(tones(srv({ state: 'disabled', stage: null }))).toEqual(ghosts)
  })
})

describe('advertisesText', () => {
  it('names what a connected server advertises', () => {
    expect(advertisesText(srv())).toEqual('tools')
    expect(advertisesText(docs())).toEqual('tools · resources · prompts')
    expect(advertisesText(srv({ tools_listed: false }))).toEqual('none')
  })
})

describe('safeAuthorizeLink', () => {
  it('accepts https without userinfo, and http only for a loopback host', () => {
    for (const url of [
      'https://linear.app/oauth/authorize?client_id=x',
      'http://127.0.0.1:9000/authorize',
      'http://localhost:9000/authorize',
      'http://[::1]:9000/authorize',
    ]) {
      expect(safeAuthorizeLink(url)).toEqual(url)
    }
    for (const url of [
      'http://linear.app/oauth/authorize',
      // Split so the pre-push secret scanner does not read these test URLs as credentials.
      'https://user:pw' + '@linear.app/x',
      'https://user' + '@linear.app/x',
      'http://user@127.0.0.1/x',
      'http://127.0.0.1.evil.example/x',
      'javascript:alert(1)',
      'not a url',
      '',
    ]) {
      expect(safeAuthorizeLink(url)).toEqual(null)
    }
  })
})

describe('countdown', () => {
  const now = 1_800_000_000_000

  it('counts minutes and seconds down to zero', () => {
    expect(countdown(now + 582_000, now)).toEqual('9:42')
    expect(countdown(now + 600_000, now)).toEqual('10:00')
    expect(countdown(now + 4_100, now)).toEqual('0:05')
    expect(countdown(now - 1, now)).toEqual('0:00')
  })
})

describe('signInStatus', () => {
  it('says each step of a sign-in once, and nothing otherwise', () => {
    expect(signInStatus('starting')).toBe('Starting sign-in')
    expect(signInStatus('waiting')).toBe('Waiting for sign-in. Open the sign-in page.')
    expect(signInStatus('finishing')).toBe('Finishing sign-in')
    for (const phase of ['idle', 'resumed', 'failed'] as const) expect(signInStatus(phase)).toBe('')
  })
})

describe('signInView', () => {
  const now = 1_800_000_000_000
  const pending = { pending: true, expires_at: now + 582_000 }
  const held: LoginStart = {
    authorize_url: 'https://linear.app/oauth/authorize?state=s',
    redirect_uri: 'http://127.0.0.1:3111/mcp-client/oauth/callback',
    expires_at: now + 582_000,
  }
  const view = (over: Partial<Parameters<typeof signInView>[0]>) =>
    signInView({ login: null, held: null, busy: null, error: null, now, ...over })

  it('shows the link only while this tab holds the start reply', () => {
    expect(view({ login: pending, held })).toEqual({
      phase: 'waiting',
      link: held.authorize_url,
      canCopyLink: true,
      timer: true,
      paste: true,
      startAgain: false,
    })
  })

  it('resumes after a reload with the timer, the paste field and Start again, and no link', () => {
    expect(view({ login: pending })).toEqual({
      phase: 'resumed',
      link: null,
      canCopyLink: false,
      timer: true,
      paste: true,
      startAgain: true,
    })
  })

  it('offers no link the browser should not open', () => {
    expect(view({ login: pending, held: { ...held, authorize_url: 'http://linear.app/x' } })).toMatchObject({
      phase: 'waiting',
      link: null,
      canCopyLink: false,
    })
  })

  it('covers the other phases', () => {
    expect(view({}).phase).toEqual('idle')
    expect(view({ busy: 'start' }).phase).toEqual('starting')
    expect(view({ login: pending, held, busy: 'finish' }).phase).toEqual('finishing')
    expect(view({ error: 'state mismatch' })).toMatchObject({ phase: 'failed', link: null, timer: false })
    expect(view({ login: { pending: true, expires_at: now - 1 }, held })).toMatchObject({ phase: 'failed', link: null })
  })
})

describe('processFacts', () => {
  const now = 1_800_000_000_000
  const values = (facts: { label: string; value: string }[]) => facts.map((fact) => fact.value)

  it('lists the facts of a running process', () => {
    const facts = processFacts(
      {
        pid: 48213,
        started_at: now - 8_040_000,
        restarts: 2,
        restart_budget: 5,
        next_restart_at: null,
        last_exit: { code: 1, signal: null },
      },
      now,
    )
    expect(facts.map((fact) => fact.label)).toEqual(['State', 'Pid', 'Uptime', 'Restarts', 'Next restart', 'Last exit'])
    expect(values(facts)).toEqual(['running', '48213', '2h 14m', '2 of 5', '—', 'code 1'])
  })

  it('lists the facts of one waiting to restart', () => {
    expect(
      values(
        processFacts(
          {
            pid: null,
            started_at: null,
            restarts: 5,
            restart_budget: 5,
            next_restart_at: now + 7_200,
            last_exit: { code: null, signal: 9 },
          },
          now,
        ),
      ),
    ).toEqual(['restarting', '—', '—', '5 of 5', 'in 8 s', 'signal 9'])
    expect(
      processFacts(
        { pid: null, started_at: null, restarts: 0, restart_budget: 5, next_restart_at: null, last_exit: null },
        now,
      )[0],
    ).toEqual({ label: 'State', value: 'stopped' })
  })
})

describe('signInControls', () => {
  it('shows sign-in only for a server that signs in, with Connect or Sign out when idle', () => {
    expect(signInControls(srv({ state: 'ready', auth: 'bearer' }), 'failed')).toEqual({
      show: false,
      connect: false,
      signOut: false,
    })
    expect(signInControls(srv({ state: 'needs_login', auth: 'oauth' }), 'idle')).toEqual({
      show: true,
      connect: true,
      signOut: false,
    })
    expect(signInControls(srv({ state: 'ready', auth: 'oauth' }), 'idle')).toEqual({
      show: true,
      connect: false,
      signOut: true,
    })
    expect(
      signInControls(
        srv({ state: 'needs_login', auth: 'oauth', login: { pending: true, expires_at: NOW + 582_000 } }),
        'resumed',
      ),
    ).toEqual({ show: true, connect: false, signOut: false })
  })
})

const item = (name: string, source: string) => ({ id: L(name), source })

describe('accessConfirm', () => {
  const access = [
    fa('list_issues', 'read', 'server_read', 'tool'),
    fa('get_issue', 'read', 'name', 'tool'),
    fa('instructions', 'read', 'definition', 'instructions'),
    fa('resources::read', 'read', 'definition', 'resource'),
    fa('prompts::code_review', 'read', 'definition', 'prompt'),
    fa('save_issue', 'write', 'server_write', 'tool'),
  ]
  const status = (rules: unknown[]) => gateStatus({ gate: { installed: true, rules }, server: srv({}), access })

  it('lists what will be allowed, name-guessed tools first', () => {
    expect(accessConfirm('linear', status([]), access)).toEqual({
      title: 'Let agents read linear without asking?',
      body: 'The approval gate will allow these 5 functions. Every other function of this server still waits for a person.',
      groups: [
        {
          label: 'Tools',
          items: [
            item('get_issue', 'Guessed from the name'),
            item('instructions', 'Reads by definition'),
            item('list_issues', 'Marked read-only by the server'),
          ],
        },
        { label: 'Resources', items: [item('resources::read', 'Reads by definition')] },
        { label: 'Prompts', items: [item('prompts::code_review', 'Reads by definition')] },
      ],
    })
  })

  it('lists Added and Removed for an update', () => {
    const update = status([rule(L('get_issue')), rule(L('save_issue')), rule(L('gone'))])
    expect(accessConfirm('linear', update, access).groups).toEqual([
      {
        label: 'Added',
        items: [
          item('instructions', 'Reads by definition'),
          item('list_issues', 'Marked read-only by the server'),
          item('prompts::code_review', 'Reads by definition'),
          item('resources::read', 'Reads by definition'),
        ],
      },
      {
        label: 'Removed',
        items: [item('gone', 'No longer listed by the server'), item('save_issue', 'Marked as writing by the server')],
      },
    ])
  })
})

describe('reviewAccessPanel', () => {
  const access = [
    fa('get_issue', 'read', 'name', 'tool'),
    fa('save_issue', 'write', 'server_write', 'tool'),
    fa('delete_issue', 'destructive', 'server_write', 'tool'),
  ]
  const status = (rules: unknown[], verdicts?: Record<string, Verdict>) =>
    gateStatus({ gate: { installed: true, rules }, server: srv({}), access, verdicts })

  it('names the function an agent can run without a person', () => {
    const rules = [rule(L('get_issue')), rule(L('save_issue'))]
    expect(reviewAccessPanel(status(rules), 'linear')).toBe('Agents can run `save_issue` without approval.')
    expect(reviewAccessPanel(status(rules, { [L('delete_issue')]: { verdict: 'allow' } }), 'linear')).toBe(
      'Agents can run `save_issue` and 1 more without approval.',
    )
    expect(reviewAccessPanel(status([rule(L('get_issue'))]), 'linear')).toBeNull()
  })

  it('names an id outside this server in full', () => {
    const foreign = { function: 'mcp::other::save', action: 'allow', rule_id: 'mcp-client:mcp::linear::save' }
    const glob = { function: 'mcp::linear::*', action: 'allow', rule_id: 'mcp-client:mcp::linear::all' }
    expect(reviewAccessPanel(status([foreign]), 'linear')).toBe('Agents can run `mcp::other::save` without approval.')
    expect(reviewAccessPanel(status([glob]), 'linear')).toBe('Agents can run `mcp::linear::*` without approval.')
  })
})

describe('accessAction and readOnlyEndpoint', () => {
  it('gives each state its one default button', () => {
    expect(
      (
        [
          'no_gate',
          'not_connected',
          'no_rules',
          'present',
          'missing',
          'stale_harmless',
          'stale_unsafe',
          'overridden_read',
          'overridden_write',
          'unreadable',
        ] as const
      ).map(accessAction),
    ).toEqual([null, 'remove', 'allow', 'remove', 'update', 'update', 'update', 'settings', 'settings', 'retry'])
  })

  it('offers the read-only endpoint for Linear only', () => {
    expect(readOnlyEndpoint('https://mcp.linear.app/mcp')).toBe('https://mcp.linear.app/mcp/readonly')
    for (const target of [
      'https://mcp.linear.app/mcp/readonly',
      'https://mcp.linear.app/mcp/',
      'https://mcp.example.com/mcp',
      'secret://LINEAR_URL',
      'npx x',
    ]) {
      expect(readOnlyEndpoint(target)).toBeNull()
    }
  })
})

describe('accessLists', () => {
  it('counts what an agent can run without a person, in the alert states too', () => {
    const access = [
      fa('get_issue', 'read', 'name'),
      fa('list_issues', 'read', 'server_read'),
      fa('save_issue', 'write', 'server_write'),
      fa('delete_issue', 'destructive', 'server_write'),
    ]
    const split = (rules: unknown[], verdicts?: Record<string, Verdict>) => {
      const st = gateStatus({ gate: { installed: true, rules }, server: srv({}), access, verdicts })
      const out = accessLists(st, 'linear', rules, access, verdicts)
      return { kind: st.kind, allowed: out.allowed.map((i) => i.id), ask: out.ask.map((i) => i.id) }
    }
    const reads = [rule(L('get_issue')), rule(L('list_issues'))]
    expect(split([...reads, rule(L('save_issue'))])).toEqual({
      kind: 'stale_unsafe',
      allowed: [L('get_issue'), L('list_issues'), L('save_issue')],
      ask: [L('delete_issue')],
    })
    expect(split(reads, { [L('delete_issue')]: { verdict: 'allow' } })).toEqual({
      kind: 'overridden_write',
      allowed: [L('get_issue'), L('list_issues'), L('delete_issue')],
      ask: [L('save_issue')],
    })
    expect(split(reads, { [L('get_issue')]: { verdict: 'deny', reason: 'denied by permission rule no-get' } })).toEqual(
      {
        kind: 'overridden_read',
        allowed: [L('list_issues')],
        ask: [L('get_issue'), L('save_issue'), L('delete_issue')],
      },
    )
  })

  it('counts only the destructive functions that still ask', () => {
    const access = [fa('get_issue', 'read', 'server_read'), fa('delete_issue', 'destructive', 'server_write')]
    const count = (rules: unknown[]) => {
      const st = gateStatus({ gate: { installed: true, rules }, server: srv({}), access })
      return [st.kind, accessLists(st, 'linear', rules, access).askDestructive]
    }
    expect(count([rule(L('get_issue'))])).toEqual(['present', 1])
    // A destructive id left in the block runs without a person, so it is not among those that ask.
    expect(count([rule(L('get_issue')), rule(L('delete_issue'))])).toEqual(['stale_unsafe', 0])
  })

  it('splits what the counts cover, and lists a block it cannot judge as stored', () => {
    const access = [
      fa('get_issue', 'read', 'name'),
      fa('list_issues', 'read', 'server_read'),
      fa('save_issue', 'write', 'server_write'),
    ]
    const rules = [rule(L('get_issue')), rule(L('gone')), rule(L('get_issue'))]
    const verdicts: Record<string, Verdict> = {
      [L('get_issue')]: { verdict: 'deny', reason: 'denied by permission rule no-get' },
      [L('save_issue')]: { verdict: 'allow' },
    }
    const st = gateStatus({ gate: { installed: true, rules }, server: srv({}), access, verdicts })
    expect(accessLists(st, 'linear', rules, access, verdicts)).toEqual({
      block: 2,
      allowed: [item('save_issue', 'Marked as writing by the server')],
      ask: [item('get_issue', 'Guessed from the name'), item('list_issues', 'Marked read-only by the server')],
      askDestructive: 0,
      toAdd: [item('list_issues', 'Marked read-only by the server')],
      toRemove: [item('gone', 'No longer listed by the server')],
      overridden: [
        item('get_issue', 'denied by permission rule no-get'),
        item('save_issue', 'Allowed by the approval gate without asking'),
      ],
    })
    const off = srv({ tools_listed: false, tools: [] })
    const unlisted = gateStatus({ gate: { installed: true, rules }, server: off, access: [] })
    expect(accessLists(unlisted, 'linear', rules, [])).toEqual({
      block: 2,
      allowed: [item('get_issue', ''), item('gone', '')],
      ask: [],
      askDestructive: 0,
      toAdd: [],
      toRemove: [],
      overridden: [],
    })
  })
})

describe('entryError', () => {
  it.each([
    ['/mcpServers/linear: set `url` or `command`, not both', 'set `url` or `command`, not both'],
    ['/mcpServers/linear/url: use `env://VAR` instead of `${VAR}`', 'url: use `env://VAR` instead of `${VAR}`'],
    // another entry's pointer, or a name that only starts the same, is kept as written
    ['/mcpServers/linear-app/url: bad', '/mcpServers/linear-app/url: bad'],
    ['url is not a valid URL', 'url is not a valid URL'],
  ])('%s', (message, shown) => {
    expect(entryError('linear', message)).toBe(shown)
  })

  it('heads the panel without the pointer too', () => {
    const s = server({ state: 'config_error', error: err('invalid_config', '/mcpServers/linear/url: bad') })
    expect(presentServer(s, 0).panel?.headline).toBe('url: bad')
  })
})
