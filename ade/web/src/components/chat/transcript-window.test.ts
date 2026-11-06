import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionCallMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import {
  agentMovedPast,
  type FlatTimelineRow,
  flattenTimelineRows,
  isHistoryPrepend,
  isLiveRow,
  isProseRow,
  rowIndexByEntryId,
  rowsWithSpacers,
  WINDOW_MIN_ROWS,
  WINDOW_OPENING_ROWS,
  windowedRowIndexes,
} from './transcript-window'

const assistant = (
  id: string,
  extra: Partial<AssistantMessage> = {},
): AssistantMessage => ({
  id,
  role: 'assistant',
  content: id,
  stopReason: 'end',
  createdAt: 0,
  ...extra,
})

const user = (id: string): UserMessage =>
  ({ id, role: 'user', content: id, createdAt: 0 }) as UserMessage

const call = (
  id: string,
  extra: Partial<FunctionCallMessage> = {},
): FunctionCallMessage => ({
  id,
  role: 'function-call',
  functionId: 'shell::run',
  input: {},
  output: { ok: true },
  createdAt: 0,
  ...extra,
})

const thought = (id: string, streaming: boolean): ThoughtMessage =>
  ({
    id,
    role: 'thought',
    content: id,
    streaming,
    createdAt: 0,
  }) as ThoughtMessage

const groupRow = (
  id: string,
  calls: FunctionCallMessage[],
  summary?: AssistantMessage,
) => ({
  kind: 'activity-group' as const,
  id,
  items: calls.map((message) => ({
    kind: 'function-call' as const,
    id: message.id,
    message,
  })),
  summary,
})

const messageRow = (
  message: AssistantMessage | UserMessage | ThoughtMessage,
) => ({
  kind: 'message' as const,
  message,
})

const flat = (
  rows: Parameters<typeof flattenTimelineRows>[0],
  settling: string[] = [],
): FlatTimelineRow[] => flattenTimelineRows(rows, new Set(settling))

describe('flattenTimelineRows', () => {
  it('gives a batch and its summary one row each, in stack order', () => {
    const rows = flat([
      messageRow(user('u')),
      groupRow('g', [call('c1')], assistant('summary')),
      messageRow(assistant('a')),
    ])
    expect(rows.map((row) => row.key)).toEqual(['u', 'g', 'summary', 'a'])
    expect(rows.map((row) => row.kind)).toEqual([
      'message',
      'activity-group',
      'summary',
      'message',
    ])
  })

  it('keeps the stagger of the row each came from', () => {
    const rows = flat([
      messageRow(user('u')),
      groupRow('g', [call('c1')], assistant('summary')),
    ])
    expect(rows.map((row) => row.staggerIndex)).toEqual([0, 1, 2])
  })

  it('keeps a merged batch under the key it is mounted with', () => {
    const merged = groupRow('older', [
      call('older', { functionCallId: 'fc_older' }),
      call('head', { functionCallId: 'fc_head' }),
    ])
    const [row] = flattenTimelineRows([merged], new Set(), new Set(['fc_head']))
    expect(row.key).toBe('fc_head')
  })

  it('keys a batch by its first item when nothing about it is known', () => {
    const [row] = flat([groupRow('g', [call('c1')])])
    expect(row.key).toBe('g')
  })

  it('drops a finished thought unless it is still settling', () => {
    const rows = flat(
      [
        messageRow(thought('gone', false)),
        messageRow(thought('settling', false)),
        messageRow(thought('live', true)),
      ],
      ['settling'],
    )
    expect(rows.map((row) => row.key)).toEqual(['settling', 'live'])
  })
})

describe('isLiveRow', () => {
  const liveOf = (row: Parameters<typeof flattenTimelineRows>[0][number]) =>
    isLiveRow(flat([row])[0])

  it.each([
    ['a running call', { running: true }],
    ['a call waiting on approval', { pendingApproval: true }],
  ])('pins a batch holding %s', (_name, extra) => {
    expect(liveOf(groupRow('g', [call('done'), call('c', extra)]))).toBe(true)
  })

  it('pins a batch holding an ask card, answered or not', () => {
    const ask = call('c', { functionId: 'harness::ask' })
    expect(liveOf(groupRow('g', [call('done'), ask]))).toBe(true)
  })

  it('lets a settled batch and settled prose leave the window', () => {
    expect(liveOf(groupRow('g', [call('c1'), call('c2')]))).toBe(false)
    expect(liveOf(messageRow(assistant('a')))).toBe(false)
    expect(liveOf(messageRow(user('u')))).toBe(false)
  })

  it('pins a reply being written and any thought still shown', () => {
    expect(liveOf(messageRow(assistant('a', { streaming: true })))).toBe(true)
    expect(liveOf(messageRow(thought('t', true)))).toBe(true)
  })

  it('pins a summary still being written, apart from its settled batch', () => {
    const rows = flat([
      groupRow('g', [call('c')], assistant('s', { streaming: true })),
    ])
    expect(rows.map(isLiveRow)).toEqual([false, true])
  })
})

describe('isProseRow', () => {
  it('counts assistant replies and summaries, nothing else', () => {
    const rows = flat([
      groupRow('g', [call('c')], assistant('summary')),
      messageRow(assistant('a')),
      messageRow(user('u')),
    ])
    expect(rows.map(isProseRow)).toEqual([false, true, true, false])
    expect(isProseRow(undefined)).toBe(false)
  })
})

describe('rowIndexByEntryId', () => {
  it('finds a call inside a batch and an absorbed wake notification', () => {
    const notification = user('wake')
    const record = {
      id: 'record',
      role: 'system',
      content: '',
      createdAt: 0,
    } as never
    const rows = flat([
      messageRow(user('u')),
      {
        kind: 'activity-group',
        id: 'g',
        items: [
          { kind: 'function-call', id: 'c1', message: call('c1') },
          {
            kind: 'observer-activity',
            id: 'o1',
            message: record,
            notification,
          },
        ],
        summary: assistant('summary'),
      },
      messageRow(assistant('a')),
    ])
    const indexes = rowIndexByEntryId(rows)
    expect(indexes.get('u')).toBe(0)
    expect(indexes.get('g')).toBe(1)
    expect(indexes.get('c1')).toBe(1)
    expect(indexes.get('record')).toBe(1)
    expect(indexes.get('wake')).toBe(1)
    expect(indexes.get('summary')).toBe(2)
    expect(indexes.get('a')).toBe(3)
    expect(indexes.get('absent')).toBeUndefined()
  })
})

describe('agentMovedPast', () => {
  const batch = groupRow('g', [call('c1')])

  it('holds a batch at the tail while the turn runs', () => {
    expect(agentMovedPast(flat([batch]), 0, true)).toBe(false)
  })

  it('holds it through a thought, the agent working out its next call', () => {
    const rows = flat([batch, messageRow(thought('t', true))])
    expect(agentMovedPast(rows, 0, true)).toBe(false)
  })

  it('lets it go once a turn ends, whatever follows', () => {
    expect(agentMovedPast(flat([batch]), 0, false)).toBe(true)
  })

  it.each([
    ['its summary', [groupRow('g', [call('c1')], assistant('s'))]],
    ['a reply', [batch, messageRow(assistant('a'))]],
    ['a user message', [batch, messageRow(user('u'))]],
    [
      'a thought and then a reply',
      [batch, messageRow(thought('t', true)), messageRow(assistant('a'))],
    ],
  ])('lets it go once %s comes after it', (_name, rows) => {
    expect(agentMovedPast(flat(rows), 0, true)).toBe(true)
  })
})

describe('rowsWithSpacers', () => {
  const item = (index: number, start: number, end: number) => ({
    index,
    key: `k${index}`,
    start,
    end,
  })
  const render = (
    items: Parameters<typeof rowsWithSpacers>[0],
    total: number,
  ) =>
    rowsWithSpacers(
      items,
      total,
      (index) => `row ${index}`,
      (key, slot) => `${key} ${slot}`,
    )

  it('adds nothing around a list with every row mounted', () => {
    expect(render([item(0, 0, 40), item(1, 40, 80)], 80)).toEqual([
      'row 0',
      'row 1',
    ])
  })

  it('stands in for the runs above, between and below the mounted rows', () => {
    expect(
      render([item(2, 80, 120), item(3, 120, 160), item(7, 280, 320)], 400),
    ).toEqual([
      'spacer:k2 80',
      'row 2',
      'row 3',
      'spacer:k7 120',
      'row 7',
      'spacer:end 80',
    ])
  })
})

describe('isHistoryPrepend', () => {
  const list = (...ids: string[]) => ids.map((id) => user(id))

  it('is a new head with the old head still below it', () => {
    expect(isHistoryPrepend('b', list('a', 'b', 'c'))).toBe(true)
  })

  it('is not an append, a first page, or a replaced transcript', () => {
    expect(isHistoryPrepend('a', list('a', 'b'))).toBe(false)
    expect(isHistoryPrepend(undefined, list('a'))).toBe(false)
    expect(isHistoryPrepend('z', list('a', 'b'))).toBe(false)
  })
})

describe('windowedRowIndexes', () => {
  const range = (count: number, startIndex: number, endIndex: number) => ({
    count,
    startIndex,
    endIndex,
    overscan: 1,
  })
  const none = { opening: false, frozen: null, pinned: [] }

  it('mounts every row of a short chat', () => {
    expect(windowedRowIndexes(range(WINDOW_MIN_ROWS, 5, 6), none)).toHaveLength(
      WINDOW_MIN_ROWS,
    )
  })

  it('mounts the viewport, its overscan and the tail of a long one', () => {
    expect(windowedRowIndexes(range(100, 50, 51), none)).toEqual([
      49, 50, 51, 52, 99,
    ])
  })

  it('merges pinned rows in order, once each, ignoring stale ones', () => {
    expect(
      windowedRowIndexes(range(100, 50, 51), {
        ...none,
        pinned: [3, 51, 500, -1],
      }),
    ).toEqual([3, 49, 50, 51, 52, 99])
  })

  it('holds the tail while opening, whatever the offset says', () => {
    const indexes = windowedRowIndexes(range(100, 0, 1), {
      ...none,
      opening: true,
    })
    expect(indexes).toHaveLength(WINDOW_OPENING_ROWS)
    expect(indexes[0]).toBe(100 - WINDOW_OPENING_ROWS)
    expect(indexes.at(-1)).toBe(99)
  })

  it('holds the rows on screen through a prepend commit', () => {
    expect(
      windowedRowIndexes(range(120, 2, 3), {
        ...none,
        frozen: [70, 71],
        pinned: [5],
      }),
    ).toEqual([5, 70, 71, 119])
  })

  it('holds the rows on screen through a prepend on a short chat too', () => {
    expect(
      windowedRowIndexes(range(WINDOW_MIN_ROWS - 2, 0, 3), {
        ...none,
        frozen: [4, 5],
      }),
    ).toEqual([4, 5, WINDOW_MIN_ROWS - 3])
  })

  it('lets the opening tail win over a prepend while hydrating', () => {
    const indexes = windowedRowIndexes(range(100, 0, 1), {
      opening: true,
      frozen: [0],
      pinned: [],
    })
    expect(indexes).not.toContain(0)
  })
})
