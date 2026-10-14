import { describe, expect, it } from 'vitest'
import {
  agentIdError,
  numberError,
  parseProcesses,
  parseWholeNumber,
  patternError,
  readTmuxConfiguration,
  withAgent,
  withDefaultAgents,
  withNewAgent,
  withNumber,
  withoutAgent,
  withText,
} from '../model'

const stored = {
  tmux_binary: 'tmux',
  poll_interval_ms: 1000,
  future_knob: { keep: true },
  agents: [
    {
      id: 'claude',
      label: 'Claude Code',
      process: ['claude'],
      working_patterns: ['esc to interrupt'],
      waiting_patterns: ['Do you want to'],
      color: 'teal',
    },
  ],
}

describe('readTmuxConfiguration', () => {
  it('reads agents without materializing defaults', () => {
    const model = readTmuxConfiguration(stored)
    expect(model.agentsDefault).toBe(false)
    expect(model.agents[0]).toMatchObject({ id: 'claude', processes: ['claude'], working: ['esc to interrupt'] })
    expect(readTmuxConfiguration({}).agentsDefault).toBe(true)
    expect(readTmuxConfiguration({ agents: 'custom' }).agentsOpaque).toBe(true)
    expect(readTmuxConfiguration('opaque').hasOpaqueRoot).toBe(true)
  })
})

describe('edits', () => {
  it('sets and clears scalar keys while preserving unknown ones', () => {
    const cleared = withNumber(stored, 'poll_interval_ms', null) as Record<string, unknown>
    expect(cleared).not.toHaveProperty('poll_interval_ms')
    expect(cleared.future_knob).toEqual({ keep: true })
    const named = withText(stored, 'socket_name', 'work')
    expect(named).toHaveProperty('socket_name', 'work')
    expect(withText(named, 'socket_name', '  ')).not.toHaveProperty('socket_name')
  })

  it('patches one agent and keeps its unknown fields', () => {
    const next = withAgent(stored, 0, { waiting: ['❯ 1\\. Yes'] }) as typeof stored
    expect(next.agents[0].waiting_patterns).toEqual(['❯ 1\\. Yes'])
    expect(next.agents[0].color).toBe('teal')
    expect(next.agents[0].process).toEqual(['claude'])
  })

  it('adds and removes agents, and can go back to the built-in list', () => {
    const added = withNewAgent(stored, 'aider') as typeof stored
    expect(added.agents.map((agent) => agent.id)).toEqual(['claude', 'aider'])
    const removed = withoutAgent(added, 0) as typeof stored
    expect(removed.agents.map((agent) => agent.id)).toEqual(['aider'])
    expect(withDefaultAgents(removed)).not.toHaveProperty('agents')
  })

  it('leaves an opaque root alone', () => {
    expect(withText('opaque', 'tmux_binary', 'x')).toBe('opaque')
    expect(withNewAgent('opaque', 'x')).toBe('opaque')
  })
})

describe('field parsing', () => {
  it('commits whole numbers only', () => {
    expect(parseWholeNumber('')).toBeNull()
    expect(parseWholeNumber(' 1500 ')).toBe(1500)
    expect(parseWholeNumber('1.5')).toBeUndefined()
    expect(parseWholeNumber('abc')).toBeUndefined()
  })

  it('bounds numbers to what the worker accepts', () => {
    expect(numberError('poll_interval_ms', 50)).toMatch(/Between/)
    expect(numberError('poll_interval_ms', 1000)).toBeNull()
    expect(numberError('max_terminals', null)).toBeNull()
  })

  it('splits process names and checks patterns and ids', () => {
    expect(parseProcesses('claude, codex  node')).toEqual(['claude', 'codex', 'node'])
    expect(patternError('esc to interrupt')).toBeNull()
    expect(patternError('(')).toMatch(/regular expression/)
    expect(patternError('')).toMatch(/Empty/)
    const agents = readTmuxConfiguration({ agents: [{ id: 'a' }, { id: 'a' }, { id: 'Bad Id' }] }).agents
    expect(agentIdError(agents, 0)).toMatch(/Another/)
    expect(agentIdError(agents, 2)).toMatch(/Lowercase/)
  })
})
