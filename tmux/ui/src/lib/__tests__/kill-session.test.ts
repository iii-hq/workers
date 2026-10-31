import { describe, expect, it } from 'vitest'
import {
  clip,
  clipMiddle,
  killSessionDescription,
  MAX_NAMED_WINDOWS,
  MAX_PART_CHARS,
  MAX_RUNNING_CHARS,
  type NamedWindow,
  windowList,
  windowPart,
} from '../kill-session'
import { buildSessionTree } from '../model'
import type { Pane, Session, Window } from '../types'

function named(index: number, name: string, command: string, extra: Partial<NamedWindow> = {}): NamedWindow {
  return { label: `${index}:${name}`, command, compose: null, window: { name }, agents: [], ...extra }
}

describe('windowPart', () => {
  it('names the agent before the command it runs under', () => {
    expect(windowPart(named(0, 'claude', 'node', { agents: [{ label: 'Claude Code' }] }))).toBe(
      '0:claude (Claude Code)',
    )
  })

  it('names an iii compose stack instead of its process title', () => {
    expect(windowPart(named(1, 'iii', 'iii:c:acme', { compose: { namespace: 'acme', pid: 7, wsPort: null } }))).toBe(
      '1:iii (iii compose)',
    )
  })

  it('names the foreground command, unless it is the window name or unknown', () => {
    expect(windowPart(named(2, 'logs', 'tail'))).toBe('2:logs (tail)')
    expect(windowPart(named(3, 'vim', 'vim'))).toBe('3:vim')
    expect(windowPart(named(4, 'scratch', ' '))).toBe('4:scratch')
  })

  it('falls through a blank agent label to the command', () => {
    expect(windowPart(named(5, 'agent', 'node', { agents: [{ label: ' ' }] }))).toBe('5:agent (node)')
  })

  it('names the first agent with a label, past a blank one', () => {
    expect(windowPart(named(6, 'pair', 'node', { agents: [{ label: '' }, { label: 'Codex' }] }))).toBe('6:pair (Codex)')
  })

  it('keeps what runs in the window when its name is long', () => {
    const part = windowPart(named(7, 'x'.repeat(80), 'node', { agents: [{ label: 'Claude Code' }] }))
    expect(part.endsWith(' (Claude Code)')).toBe(true)
    expect(Array.from(part)).toHaveLength(MAX_PART_CHARS)
  })

  it('clips a long command too, and the window keeps the rest', () => {
    const part = windowPart(named(8, 'build', 'c'.repeat(60)))
    expect(part).toBe(`8:build (${'c'.repeat(MAX_RUNNING_CHARS - 1)}…)`)
  })

  it('clips a long window that runs its own name', () => {
    const name = 'v'.repeat(80)
    expect(Array.from(windowPart(named(9, name, name)))).toHaveLength(MAX_PART_CHARS)
  })
})

describe('windowList', () => {
  const many = (count: number) => Array.from({ length: count }, (_, i) => named(i, `w${i}`, `w${i}`))

  it('joins the last window with "and"', () => {
    expect(windowList(many(1))).toBe('0:w0')
    expect(windowList(many(2))).toBe('0:w0 and 1:w1')
    expect(windowList(many(3))).toBe('0:w0, 1:w1 and 2:w2')
  })

  it('names every window up to the cap', () => {
    expect(windowList(many(MAX_NAMED_WINDOWS))).toBe('0:w0, 1:w1, 2:w2, 3:w3, 4:w4 and 5:w5')
  })

  it('names one window over the cap rather than "1 more"', () => {
    expect(windowList(many(MAX_NAMED_WINDOWS + 1))).toBe('0:w0, 1:w1, 2:w2, 3:w3, 4:w4, 5:w5 and 6:w6')
  })

  it('folds the windows past the cap into a count', () => {
    expect(windowList(many(MAX_NAMED_WINDOWS + 4))).toBe('0:w0, 1:w1, 2:w2, 3:w3, 4:w4, 5:w5 and 4 more')
  })
})

describe('clip', () => {
  it('keeps a part that fits', () => {
    expect(clip('0:claude (Claude Code)')).toBe('0:claude (Claude Code)')
    expect(clip('x'.repeat(MAX_PART_CHARS))).toBe('x'.repeat(MAX_PART_CHARS))
  })

  it('cuts a longer part to the limit, ending in an ellipsis', () => {
    const cut = clip('y'.repeat(MAX_PART_CHARS + 30))
    expect(Array.from(cut)).toHaveLength(MAX_PART_CHARS)
    expect(cut.endsWith('…')).toBe(true)
  })

  it('never splits an emoji and drops the space before the ellipsis', () => {
    expect(clip('ab 🚀🚀', 4)).toBe('ab…')
    expect(clip('🚀🚀🚀🚀', 3)).toBe('🚀🚀…')
  })

  it('counts what a reader sees: a joined emoji, a flag and an accent stay whole', () => {
    expect(clip('ab👨‍👩‍👧‍👦xyz', 4)).toBe('ab👨‍👩‍👧‍👦…')
    expect(clip('ab🇧🇷cd', 4)).toBe('ab🇧🇷…')
    expect(clip('cafe\u0301 au lait', 5)).toBe('cafe\u0301…')
  })
})

describe('clipMiddle', () => {
  it('keeps a name that fits', () => {
    expect(clipMiddle('acme-api')).toBe('acme-api')
  })

  it('keeps both ends of a long name', () => {
    const cut = clipMiddle(`acme-api-staging-${'x'.repeat(60)}-eu-west`)
    expect(Array.from(cut)).toHaveLength(MAX_PART_CHARS)
    expect(cut.startsWith('acme-api-staging-')).toBe(true)
    expect(cut.endsWith('-eu-west')).toBe(true)
    expect(cut).toContain('…')
  })

  it('tells apart two names that share a long beginning', () => {
    const base = `acme-${'shared-'.repeat(10)}`
    expect(clipMiddle(`${base}api`)).not.toBe(clipMiddle(`${base}web`))
  })
})

describe('the description stays bounded', () => {
  it('clips every long window name and command, whatever the count', () => {
    const long = 'a-very-long-window-name-that-keeps-going-and-going'
    const windows = Array.from({ length: 40 }, (_, i) => named(i, `${long}-${i}`, `${long}-command`))
    const text = killSessionDescription(windows)
    const tail = ' and 34 more end with every process in them, and every terminal attached to the session — yours included — detaches.'
    expect(text.endsWith(tail)).toBe(true)
    const parts = text.slice(0, -tail.length).split(', ')
    expect(parts).toHaveLength(MAX_NAMED_WINDOWS)
    for (const part of parts) expect(Array.from(part).length).toBeLessThanOrEqual(MAX_PART_CHARS)
    expect(Array.from(text).length).toBeLessThanOrEqual(MAX_NAMED_WINDOWS * (MAX_PART_CHARS + 2) + tail.length)
  })
})

describe('killSessionDescription', () => {
  it('leads with the windows that end, in one sentence', () => {
    expect(killSessionDescription([named(0, 'editor', 'vim'), named(1, 'dev', 'npm'), named(2, 'logs', 'tail')])).toBe(
      '0:editor (vim), 1:dev (npm) and 2:logs (tail) end with every process in them, and every terminal attached to the session — yours included — detaches.',
    )
  })

  it('agrees with a single window', () => {
    expect(killSessionDescription([named(0, 'zsh', 'zsh')])).toBe(
      '0:zsh ends with every process in it, and every terminal attached to the session — yours included — detaches.',
    )
  })

  it('counts the windows past the cap in the sentence', () => {
    const windows = Array.from({ length: MAX_NAMED_WINDOWS + 2 }, (_, i) => named(i, `w${i}`, `w${i}`))
    expect(killSessionDescription(windows)).toBe(
      '0:w0, 1:w1, 2:w2, 3:w3, 4:w4, 5:w5 and 2 more end with every process in them, and every terminal attached to the session — yours included — detaches.',
    )
  })

  it('falls back to the general sentence without windows', () => {
    expect(killSessionDescription([])).toBe(
      'Every window and process in it ends, and every terminal attached to the session — yours included — detaches.',
    )
  })

  it('reads the rows the session tree builds', () => {
    const pane = (overrides: Partial<Pane>): Pane => ({
      id: '%1',
      index: 0,
      active: true,
      pid: 10,
      command: 'bash',
      cwd: '/tmp/acme/api',
      title: '',
      width: 80,
      height: 24,
      dead: false,
      agent: null,
      compose: null,
      ports: [],
      ...overrides,
    })
    const window = (index: number, name: string, panes: Pane[]): Window => ({
      id: `@${index}`,
      index,
      name,
      active: index === 0,
      activity_at_ms: 0,
      bell: false,
      zoomed: false,
      panes,
    })
    const session: Session = {
      id: '$1',
      name: 'api',
      created_at_ms: 0,
      activity_at_ms: 0,
      attached: 0,
      console_attached: 1,
      windows: [
        window(0, 'claude', [
          pane({ agent: { id: 'claude', label: 'Claude Code', pid: 11, state: 'working', since_ms: 0 } }),
        ]),
        window(1, 'iii', [pane({ id: '%2', command: 'iii:c:acme', compose: { namespace: 'acme', pid: 12 } })]),
        window(2, 'shell', [pane({ id: '%3' })]),
      ],
    }
    const tmux = {
      available: true,
      path: '/usr/bin/tmux',
      version: '3.6',
      socket_path: '/tmp/tmux-1000/acme-demo',
      server_running: true,
      error: null,
    }
    const [row] = buildSessionTree({ tmux, sessions: [session], generated_at_ms: 1 }, null, '$1')
    expect(killSessionDescription(row.windows)).toBe(
      '0:claude (Claude Code), 1:iii (iii compose) and 2:shell (bash) end with every process in them, and every terminal attached to the session — yours included — detaches.',
    )
  })
})
