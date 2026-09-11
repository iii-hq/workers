import { describe, expect, it } from 'vitest'
import type { Conversation, Message, SubagentAppearance } from '@/types/chat'
import {
  buildActiveSubagentChipModel,
  collectSubagentDescendants,
  deriveSubagentVisualStatus,
  resolveSubagentAppearance,
} from './active-subagents'

function conversation(
  id: string,
  overrides: Partial<Conversation> = {},
): Conversation {
  return {
    id,
    title: id,
    model: null,
    messages: [],
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  }
}

function userMessage(id = 'message'): Message {
  return { id, role: 'user', content: 'task', createdAt: 1 }
}

describe('collectSubagentDescendants', () => {
  it('walks nested children in stable breadth-first order', () => {
    const result = collectSubagentDescendants(
      [
        conversation('grandchild', {
          parentId: 'first',
          createdAt: 1,
        }),
        conversation('second', { parentId: 'root', createdAt: 2 }),
        conversation('unrelated', { parentId: 'other' }),
        conversation('first', { parentId: 'root', createdAt: 1 }),
      ],
      'root',
    )

    expect(
      result.descendants.map(({ conversation: child, relativeDepth }) => [
        child.id,
        relativeDepth,
      ]),
    ).toEqual([
      ['first', 1],
      ['second', 1],
      ['grandchild', 2],
    ])
    expect(result.truncated).toBe(false)
  })

  it('breaks corrupt parent cycles and reports traversal limits', () => {
    const cyclic = collectSubagentDescendants(
      [
        conversation('root', { parentId: 'grandchild' }),
        conversation('child', { parentId: 'root' }),
        conversation('grandchild', { parentId: 'child' }),
      ],
      'root',
    )
    expect(
      cyclic.descendants.map(({ conversation: child }) => child.id),
    ).toEqual(['child', 'grandchild'])

    const bounded = collectSubagentDescendants(
      Array.from({ length: 10 }, (_, index) =>
        conversation(`child-${index}`, {
          parentId: 'root',
          createdAt: index,
        }),
      ),
      'root',
      { maxDescendants: 3 },
    )
    expect(bounded.descendants).toHaveLength(3)
    expect(bounded.truncated).toBe(true)
  })

  it('stops descending at the configured depth', () => {
    const result = collectSubagentDescendants(
      [
        conversation('child', { parentId: 'root' }),
        conversation('grandchild', { parentId: 'child' }),
      ],
      'root',
      { maxDepth: 1 },
    )

    expect(
      result.descendants.map(({ conversation: child }) => child.id),
    ).toEqual(['child'])
    expect(result.truncated).toBe(true)
  })
})

describe('deriveSubagentVisualStatus', () => {
  it.each([
    [
      'idle without a transcript',
      conversation('child', { status: 'idle' }),
      'queued',
    ],
    [
      'idle with transcript history',
      conversation('child', {
        status: 'idle',
        messages: [userMessage()],
      }),
      'waiting',
    ],
    [
      'queued phase reason',
      conversation('child', {
        status: 'working',
        statusReason: 'queued for dispatch',
      }),
      'queued',
    ],
    [
      'provider wait phase',
      conversation('child', {
        status: 'working',
        statusReason: 'waiting for model',
      }),
      'waiting',
    ],
    [
      'working session',
      conversation('child', { status: 'working' }),
      'working',
    ],
    [
      'completed session',
      conversation('child', { status: 'done' }),
      'completed',
    ],
    ['failed session', conversation('child', { status: 'error' }), 'failed'],
    [
      'failure diagnostic mentioning a stop',
      conversation('child', {
        status: 'error',
        statusReason: 'provider stopped responding',
      }),
      'failed',
    ],
    [
      'accepted stop',
      conversation('child', {
        status: 'working',
        statusReason: 'stopping',
      }),
      'stopped',
    ],
    [
      'persisted stop marker',
      conversation('child', {
        status: 'done',
        messages: [
          {
            id: 'e_turn_stopped',
            role: 'system',
            kind: 'notice',
            content: 'stopped by user',
            createdAt: 1,
          },
        ],
      }),
      'stopped',
    ],
  ] as const)('maps %s to %s', (_label, child, expected) => {
    expect(deriveSubagentVisualStatus(child, 'connected')).toBe(expected)
  })

  it('uses connection loss only for non-terminal sessions', () => {
    expect(
      deriveSubagentVisualStatus(
        conversation('active', { status: 'working' }),
        'reconnecting',
      ),
    ).toBe('disconnected')
    expect(
      deriveSubagentVisualStatus(
        conversation('done', { status: 'done' }),
        'disconnected',
      ),
    ).toBe('completed')
  })

  it('does not let an old stopped transcript marker hide resumed work', () => {
    expect(
      deriveSubagentVisualStatus(
        conversation('resumed', {
          status: 'working',
          messages: [
            {
              id: 'e_previous_stopped',
              role: 'system',
              content: 'previous turn stopped',
              createdAt: 1,
            },
          ],
        }),
        'connected',
      ),
    ).toBe('working')
  })
})

describe('resolveSubagentAppearance', () => {
  it('normalizes a harness-provided enum appearance', () => {
    const result = resolveSubagentAppearance(
      conversation('child', {
        subagentAppearance: {
          name: '  Frontend\nAgent  ',
          icon: 'code',
          color: 'purple',
        },
      }),
    )

    expect(result).toEqual({
      name: 'Frontend Agent',
      icon: 'code',
      color: 'purple',
    })
  })

  it('falls back safely when runtime metadata is blank or outside the enums', () => {
    const invalid = {
      name: ' ',
      icon: 'rocket',
      color: 'chartreuse',
    } as unknown as SubagentAppearance
    expect(
      resolveSubagentAppearance(
        conversation('child', {
          title: 'Explore authentication',
          subagentAppearance: invalid,
        }),
      ),
    ).toEqual({
      name: 'Explore authentication',
      icon: 'agent',
      color: 'neutral',
    })
  })
})

describe('buildActiveSubagentChipModel', () => {
  it('does not let old terminal descendants hide a later active child', () => {
    const terminal = Array.from({ length: 64 }, (_, index) =>
      conversation(`done-${index}`, {
        parentId: 'root',
        status: 'done',
        createdAt: index + 1,
      }),
    )
    const active = conversation('still-working', {
      parentId: 'root',
      status: 'working',
      createdAt: 100,
    })

    const model = buildActiveSubagentChipModel(
      [...terminal, active],
      'root',
      'connected',
    )

    expect(model.active.map((item) => item.sessionId)).toContain(
      'still-working',
    )
  })

  it('keeps only active chips, limits them, and summarizes terminal children', () => {
    const model = buildActiveSubagentChipModel(
      [
        conversation('working', { parentId: 'root', status: 'working' }),
        conversation('queued', { parentId: 'root', status: 'idle' }),
        conversation('waiting', {
          parentId: 'root',
          status: 'working',
          statusReason: 'waiting for model',
        }),
        conversation('completed', { parentId: 'root', status: 'done' }),
        conversation('failed', { parentId: 'root', status: 'error' }),
        conversation('stopped', {
          parentId: 'root',
          status: 'done',
          messages: [
            {
              id: 'e_stopped',
              role: 'system',
              content: 'stopped by user',
              createdAt: 1,
            },
          ],
        }),
      ],
      'root',
      'connected',
      { maxVisible: 2 },
    )

    expect(model.active.map(({ status }) => status)).toEqual([
      'queued',
      'waiting',
    ])
    expect(model.omittedActive).toBe(1)
    expect(model.terminal).toEqual({
      completed: 1,
      failed: 1,
      stopped: 1,
      total: 3,
    })
  })
})
