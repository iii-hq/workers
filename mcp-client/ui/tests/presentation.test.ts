import { describe, expect, it } from 'vitest'
import {
  announcements,
  blocksOf,
  byName,
  connectionRowText,
  countLabel,
  drawsWhole,
  elapsedText,
  filterRows,
  firstVariable,
  functionRows,
  highlightable,
  jsonText,
  matchesQuery,
  messagesOf,
  metaLine,
  middleTruncate,
  narrowLevel,
  pageState,
  pathHeads,
  presentServer,
  processRowText,
  resourceMeta,
  resultSummary,
  rowGroups,
  runConfirm,
  STAGES,
  STATE_WORD,
  searchNote,
  skippedText,
  statusBarText,
} from '../src/presentation'
import type { EngineFunction, Item, Server, Skipped } from '../src/types'
import type { ServersData } from '../src/use-servers'
import { docs, list, NOW, server, stdio } from './fixtures/servers'

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
    [server({ auth: 'oauth' }), 'ok', false, 'mcp.linear.app · Signed in', null],
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

describe('metaLine', () => {
  const schema = (n: number, required: string[] = []) => ({
    type: 'object',
    properties: Object.fromEntries(Array.from({ length: n }, (_, i) => [`p${i}`, { type: 'string' }])),
    required,
  })

  it('counts inputs for tools and resource functions', () => {
    expect(metaLine('mcp::linear::list_issues', schema(12), 'tools')).toBe(
      'mcp::linear::list_issues · 12 inputs, none required',
    )
    expect(metaLine('mcp::linear::save_comment', schema(3, ['p0']), 'tools')).toBe(
      'mcp::linear::save_comment · 3 inputs, 1 required',
    )
    expect(metaLine('mcp::docs::resources::read', schema(1, ['p0']), 'resources')).toBe(
      'mcp::docs::resources::read · 1 input, 1 required',
    )
    expect(metaLine('mcp::linear::whoami', schema(0), 'tools')).toBe('mcp::linear::whoami · no inputs')
  })

  it('says arguments for a prompt', () => {
    expect(metaLine('mcp::docs::prompts::code_review', schema(2, ['p0']), 'prompts')).toBe(
      'mcp::docs::prompts::code_review · 2 arguments, 1 required',
    )
    expect(metaLine('mcp::docs::prompts::summary', schema(0), 'prompts')).toBe(
      'mcp::docs::prompts::summary · no arguments',
    )
  })

  it('is the id alone until the schema is read', () => {
    expect(metaLine('mcp::linear::whoami', undefined, 'tools')).toBe('mcp::linear::whoami')
  })

  it('is the id alone when a union hides the inputs, and counts none for no schema', () => {
    const union = { anyOf: [schema(1, ['p0']), schema(2)] }
    expect(metaLine('mcp::linear::list_issues', union, 'tools')).toBe('mcp::linear::list_issues')
    expect(metaLine('mcp::linear::whoami', null, 'tools')).toBe('mcp::linear::whoami · no inputs')
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
})
