import { describe, expect, it } from 'vitest'
import {
  agentStateLabel,
  agentStateTone,
  buildSessionTree,
  compactRequest,
  findSession,
  groupPorts,
  newerSnapshot,
  pickInitialSession,
  portUrl,
  shortPath,
  subagentCountLabel,
  terminalGone,
  transcriptItemLabel,
  transcriptPollMs,
  validateCommand,
  validateDirectory,
  validateName,
} from '../model'
import type { Listener, Pane, PortsSnapshot, Session, SessionsSnapshot, TranscriptItemKind, Window } from '../types'

const tmux = {
  available: true,
  path: '/usr/bin/tmux',
  version: '3.6',
  socket_path: '/tmp/tmux-1000/default',
  server_running: true,
  error: null,
}

function listener(port: number, overrides: Partial<Listener> = {}): Listener {
  return {
    port,
    address: '0.0.0.0',
    family: 'ipv4',
    pid: 100 + port,
    process: 'http',
    worker: null,
    compose: null,
    role: 'other',
    location: null,
    ...overrides,
  }
}

function pane(overrides: Partial<Pane> = {}): Pane {
  return {
    id: '%1',
    index: 0,
    active: true,
    pid: 10,
    command: 'zsh',
    cwd: '/home/me',
    title: '',
    width: 80,
    height: 24,
    dead: false,
    agent: null,
    compose: null,
    ports: [],
    ...overrides,
  }
}

function window(overrides: Partial<Window> = {}): Window {
  return {
    id: '@1',
    index: 0,
    name: 'zsh',
    active: true,
    activity_at_ms: 0,
    bell: false,
    zoomed: false,
    panes: [pane()],
    ...overrides,
  }
}

function session(overrides: Partial<Session> = {}): Session {
  return {
    id: '$1',
    name: 'main',
    created_at_ms: 0,
    activity_at_ms: 0,
    attached: 0,
    console_attached: 0,
    windows: [window()],
    ...overrides,
  }
}

function snapshot(sessions: Session[], generated = 1): SessionsSnapshot {
  return { tmux, sessions, generated_at_ms: generated }
}

const ports: PortsSnapshot = {
  supported: true,
  error: null,
  generated_at_ms: 1,
  composes: [
    {
      namespace: 'default',
      pid: 500,
      project_dir: '/home/me/app',
      compose_file: null,
      engine: { pid: 501, ports: [49135, 3112], ws_port: 49135 },
      workers: [{ name: 'http', pid: 502, ports: [3111] }],
      location: null,
    },
  ],
  listeners: [
    listener(49135, { pid: 501, process: 'iii:e:default', compose: 'default', role: 'engine' }),
    listener(3111, { pid: 502, worker: 'http', compose: 'default', role: 'worker' }),
    listener(5432, { process: 'postgres' }),
    listener(22, { process: 'sshd', address: '::', family: 'ipv6' }),
  ],
}

describe('buildSessionTree', () => {
  it('orders sessions by name and windows by index', () => {
    const rows = buildSessionTree(
      snapshot([
        session({ id: '$2', name: 'zeta' }),
        session({
          id: '$1',
          name: 'alpha',
          windows: [window({ id: '@2', index: 1, name: 'b' }), window({ id: '@1', index: 0, name: 'a' })],
        }),
      ]),
      null,
      null,
    )
    expect(rows.map((row) => row.session.name)).toEqual(['alpha', 'zeta'])
    expect(rows[0].windows.map((row) => row.label)).toEqual(['0:a', '1:b'])
  })

  it('badges agents (waiting first), the compose engine port and the window ports', () => {
    const rows = buildSessionTree(
      snapshot([
        session({
          attached: 2,
          console_attached: 1,
          windows: [
            window({
              panes: [
                pane({
                  id: '%1',
                  agent: { id: 'codex', label: 'Codex', pid: 1, state: 'working', since_ms: 5 },
                }),
                pane({
                  id: '%2',
                  index: 1,
                  active: false,
                  agent: { id: 'claude', label: 'Claude Code', pid: 2, state: 'waiting', since_ms: 6 },
                }),
                pane({
                  id: '%3',
                  index: 2,
                  active: false,
                  command: 'iii:c:default',
                  compose: { namespace: 'default', pid: 500 },
                  ports: [listener(3111), listener(49135), listener(3111, { address: '::' })],
                }),
              ],
            }),
          ],
        }),
      ]),
      ports,
      '$1',
    )
    const [row] = rows
    expect(row.current).toBe(true)
    expect(row.externalClients).toBe(1)
    expect(row.consoleClients).toBe(1)
    expect(row.agents).toEqual({ working: 1, waiting: 1, idle: 0, subagents: 0 })
    const [win] = row.windows
    expect(win.agents.map((agent) => agent.id)).toEqual(['claude', 'codex'])
    expect(win.compose).toEqual({ namespace: 'default', pid: 500, wsPort: 49135 })
    expect(win.ports).toEqual([3111, 49135])
  })

  it('is empty before the first snapshot', () => {
    expect(buildSessionTree(null, null, null)).toEqual([])
  })
})

describe('session lookup', () => {
  const snap = snapshot([
    session({ id: '$1', name: 'work' }),
    session({ id: '$2', name: 'attached', attached: 1 }),
    session({ id: '$3', name: 'console-only', attached: 1, console_attached: 1 }),
  ])

  it('finds by id or by exact name', () => {
    expect(findSession(snap, '$2')?.name).toBe('attached')
    expect(findSession(snap, 'work')?.id).toBe('$1')
    expect(findSession(snap, 'nope')).toBeNull()
  })

  it('opens the remembered session, else the one a real terminal shows, else the first', () => {
    expect(pickInitialSession(snap, 'work')?.id).toBe('$1')
    expect(pickInitialSession(snap, 'gone')?.id).toBe('$2')
    expect(
      pickInitialSession(snapshot([session({ id: '$9', name: 'b' }), session({ id: '$8', name: 'a' })]), null)?.id,
    ).toBe('$8')
    expect(pickInitialSession(snapshot([]), 'work')).toBeNull()
  })
})

describe('groupPorts', () => {
  it('keeps compose listeners with their stack and sorts the rest by port', () => {
    const groups = groupPorts(ports)
    expect(groups.composes.map((stack) => stack.namespace)).toEqual(['default'])
    expect(groups.others.map((l) => l.port)).toEqual([22, 5432])
  })

  it('links every port but the engine websocket', () => {
    expect(portUrl({ port: 3111, role: 'worker' })).toBe('http://localhost:3111')
    expect(portUrl({ port: 49134, role: 'engine' })).toBeNull()
  })
})

describe('sub-agents', () => {
  it('lists every agent sub-agent under its window, oldest first, and counts them per session', () => {
    const rows = buildSessionTree(
      snapshot([
        session({
          windows: [
            window({
              panes: [
                pane({
                  id: '%1',
                  agent: {
                    id: 'claude',
                    label: 'Claude Code',
                    pid: 1,
                    state: 'working',
                    since_ms: 5,
                    subagents: [
                      { id: 'a2', description: 'Review the diff', kind: 'general-purpose', depth: 1, since_ms: 30 },
                      { id: 'a1', description: 'Map the parser', kind: 'Explore', depth: 2, since_ms: 10 },
                    ],
                  },
                }),
                pane({
                  id: '%2',
                  index: 1,
                  active: false,
                  agent: {
                    id: 'codex',
                    label: 'Codex',
                    pid: 2,
                    state: 'working',
                    since_ms: 6,
                    subagents: [{ id: 't1', description: 'sleep_test', kind: null, depth: 1, since_ms: 20 }],
                  },
                }),
                // A worker that predates sub-agents sends none.
                pane({
                  id: '%3',
                  index: 2,
                  active: false,
                  agent: { id: 'gemini', label: 'Gemini CLI', pid: 3, state: 'idle', since_ms: 7 },
                }),
              ],
            }),
          ],
        }),
      ]),
      null,
      null,
    )
    const [row] = rows
    expect(row.agents).toEqual({ working: 2, waiting: 0, idle: 1, subagents: 3 })
    const [win] = row.windows
    expect(win.subagents.map((sub) => [sub.key, sub.id, sub.paneId, sub.agentLabel, sub.depth])).toEqual([
      ['%1/a1', 'a1', '%1', 'Claude Code', 2],
      ['%2/t1', 't1', '%2', 'Codex', 1],
      ['%1/a2', 'a2', '%1', 'Claude Code', 1],
    ])
    expect(win.agents.map((agent) => [agent.id, agent.subagents.length])).toEqual([
      ['claude', 2],
      ['codex', 1],
      ['gemini', 0],
    ])
  })

  it('labels counts', () => {
    expect(subagentCountLabel(1)).toBe('1 sub-agent')
    expect(subagentCountLabel(3)).toBe('3 sub-agents')
  })

  it('heads transcript items', () => {
    const label = (kind: TranscriptItemKind, tool: string | null = null, error = false) =>
      transcriptItemLabel({ kind, tool, error }, 'Claude Code')
    expect(label('user')).toBe('Message from Claude Code')
    expect(label('text')).toBe('Sub-agent')
    expect(label('thinking')).toBe('Thinking')
    expect(label('tool_call', 'Bash')).toBe('Bash')
    expect(label('tool_call')).toBe('Tool call')
    expect(label('tool_result', 'Read')).toBe('Read result')
    expect(label('tool_result', 'Read', true)).toBe('Read failed')
    expect(label('tool_result')).toBe('Tool result')
  })

  it('asks for the transcript often while the sub-agent works', () => {
    expect(transcriptPollMs(null, false)).toBe(2_000)
    expect(transcriptPollMs({ working: true }, false)).toBe(2_000)
    expect(transcriptPollMs({ working: false }, false)).toBe(10_000)
    expect(transcriptPollMs({ working: true }, true)).toBe(5_000)
  })
})

describe('agent states', () => {
  it('reads as words and maps to the status tones', () => {
    expect(agentStateLabel('waiting')).toBe('waiting for you')
    expect(agentStateTone('working')).toBe('accent')
    expect(agentStateTone('waiting')).toBe('warn')
    expect(agentStateTone('idle')).toBe('ink')
  })
})

describe('newerSnapshot', () => {
  it('never lets an older snapshot replace a newer one', () => {
    const newer = snapshot([], 20)
    const older = snapshot([session()], 10)
    expect(newerSnapshot(newer, older)).toBe(newer)
    expect(newerSnapshot(older, newer)).toBe(newer)
    expect(newerSnapshot(null, older)).toBe(older)
  })
})

describe('validation', () => {
  it('accepts tmux names and refuses what tmux would rewrite or misparse', () => {
    expect(validateName('')).toBeNull()
    expect(validateName('api server')).toBeNull()
    expect(validateName('a.b')).toMatch(/cannot contain/)
    expect(validateName('a:b')).toMatch(/cannot contain/)
    expect(validateName('-x')).toMatch(/start/)
    expect(validateName('x'.repeat(65))).toMatch(/64/)
  })

  it('wants absolute or home-relative directories', () => {
    expect(validateDirectory('')).toBeNull()
    expect(validateDirectory('~/code')).toBeNull()
    expect(validateDirectory('/srv')).toBeNull()
    expect(validateDirectory('code')).toMatch(/absolute/)
  })

  it('refuses a command that looks like a flag', () => {
    expect(validateCommand('claude --resume')).toBeNull()
    expect(validateCommand('-c foo')).toMatch(/start/)
  })

  it('drops empty optionals so the worker applies its defaults', () => {
    expect(compactRequest({ session: '$1', name: ' ', cwd: ' ~/x ', select: false, command: undefined })).toEqual({
      session: '$1',
      cwd: '~/x',
      select: false,
    })
  })
})

describe('helpers', () => {
  it('shortens a home path', () => {
    expect(shortPath('/home/me/app', '/home/me')).toBe('~/app')
    expect(shortPath('/srv/app', '/home/me')).toBe('/srv/app')
    expect(shortPath(null)).toBe('')
  })

  it('recognises a terminal that is gone', () => {
    expect(terminalGone(new Error('terminal t1 not found'))).toBe(true)
    expect(terminalGone({ code: 'x', message: 'terminal credentials are invalid' })).toBe(true)
    expect(terminalGone(new Error('timeout'))).toBe(false)
  })
})
