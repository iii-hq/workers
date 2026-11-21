import { describe, expect, it } from 'vitest'
import { countLabel, presentServer, STAGES, statusBarText } from '../src/presentation'
import type { Server } from '../src/types'
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
