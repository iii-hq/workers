import { describe, expect, it } from 'vitest'
import type { StoredSpan } from '../api/traces'
import type { TimelineSpan } from '../components/timeline/layout'
import type { SpanFilterSelection } from '../lib/spanFilters'
import {
  mergeFetchedVerdicts,
  reconcileTraceVisibility,
  rowRootFilterKeys,
} from './useSpanFilteredTraceRows'
import type { TraceListItem } from './useTraceData'

function bar(overrides: Partial<TimelineSpan> & { id: string }): TimelineSpan {
  return {
    traceId: 't-1',
    startTime: 0,
    endTime: 1,
    ...overrides,
  }
}

/** A row whose ROOT belongs to `group` — the shape `mapSpanToListItem`
 *  produces for a trace rooted in that function's dispatch span. */
function row(
  traceId: string,
  group = 'fn',
  overrides: Partial<TraceListItem> = {},
): TraceListItem {
  return {
    traceId,
    rootOperation: `enqueue ${group} → q`,
    status: 'ok',
    startTime: 0,
    spanCount: 1,
    workers: ['worker'],
    attributes: { function_id: group },
    ...overrides,
  }
}

function selection(
  overrides: Partial<SpanFilterSelection> = {},
): SpanFilterSelection {
  return {
    hiddenGroups: new Set(),
    hiddenWorkers: new Set(),
    shownInternal: new Set(),
    ...overrides,
  }
}

function stored(
  overrides: Partial<StoredSpan> & { span_id: string },
): StoredSpan {
  return {
    trace_id: 't-1',
    name: 'op',
    start_time_unix_nano: 1_000_000,
    end_time_unix_nano: 2_000_000,
    status: 'OK',
    attributes: [],
    events: [],
    links: [],
    ...overrides,
  }
}

const ids = (rows: readonly TraceListItem[]) => rows.map((r) => r.traceId)

describe('rowRootFilterKeys', () => {
  it('keys the root like the feed would: explicit function id, worker, internal family', () => {
    expect(
      rowRootFilterKeys(
        row('t-1', 'session::update-message', {
          workers: ['session-manager'],
          attributes: {
            function_id: 'session::update-message',
            'iii.tag.hidden': 'session events',
          },
        }),
      ),
    ).toEqual({
      groupKey: 'session::update-message',
      workerKey: 'session-manager',
      internalKey: 'session events',
    })
  })

  it("undoes the list's 'unknown' worker sentinel so the fallback matches the feed's", () => {
    const keys = rowRootFilterKeys(
      row('t-1', 'fn', {
        rootOperation: 'gateway.request',
        workers: ['unknown'],
        attributes: {},
      }),
    )
    expect(keys.workerKey).toBe('gateway')
  })
})

describe('reconcileTraceVisibility', () => {
  it('hides a row once every bar of its trace is hidden', () => {
    const sel = selection({
      hiddenGroups: new Set(['session::update-message']),
    })
    const bars = [
      bar({ id: 's1', groupKey: 'session::update-message' }),
      bar({ id: 's2', groupKey: 'session::update-message' }),
    ]
    const kept = reconcileTraceVisibility(
      new Map(),
      bars,
      [
        row('t-1', 'session::update-message'),
        row('t-2', 'session::update-message'),
      ],
      sel,
    )
    // t-2 has no coverage yet (hidden root, feed silent) → visible until a
    // composition read decides.
    expect(ids(kept)).toEqual(['t-2'])
  })

  it('keeps the row while any bar of its trace survives — a hidden root is not enough', () => {
    // The turn-trace shape: dispatch root in a producer-default-hidden
    // group, real work below it.
    const bars = [
      bar({ id: 'root', groupKey: 'harness::turn' }),
      bar({ id: 'step', groupKey: 'harness::turn step' }),
    ]
    const kept = reconcileTraceVisibility(
      new Map(),
      bars,
      [row('t-1', 'harness::turn')],
      selection({ hiddenGroups: new Set(['harness::turn']) }),
    )
    expect(ids(kept)).toEqual(['t-1'])
  })

  it('keeps a row whose visible ROOT outlived the feed (partial tail all hidden)', () => {
    const bars = [bar({ id: 'tail', groupKey: 'session::update-message' })]
    const kept = reconcileTraceVisibility(
      new Map(),
      bars,
      [row('t-1', 'chat.respond')],
      selection({ hiddenGroups: new Set(['session::update-message']) }),
    )
    expect(ids(kept)).toEqual(['t-1'])
  })

  it('hides worker-hidden traces', () => {
    const bars = [bar({ id: 's1', groupKey: 'fn', workerKey: 'noisy' })]
    const kept = reconcileTraceVisibility(
      new Map(),
      bars,
      [row('t-1', 'fn', { workers: ['noisy'] })],
      selection({ hiddenWorkers: new Set(['noisy']) }),
    )
    expect(kept).toEqual([])
  })

  it('hides default-hidden internal fan-outs, shows them once the family is revealed', () => {
    const bars = [bar({ id: 's1', internalKey: 'session events' })]
    const rows = [
      row('t-1', 'fn', {
        attributes: { function_id: 'fn', 'iii.tag.hidden': 'session events' },
      }),
    ]
    expect(
      ids(reconcileTraceVisibility(new Map(), bars, rows, selection())),
    ).toEqual([])
    expect(
      ids(
        reconcileTraceVisibility(
          new Map(),
          bars,
          rows,
          selection({ shownInternal: new Set(['session events']) }),
        ),
      ),
    ).toEqual(['t-1'])
  })

  it('remembers a verdict after the trace prunes out of the feed', () => {
    const verdicts = new Map<string, boolean>()
    const sel = selection({ hiddenGroups: new Set(['fn']) })
    const rows = [row('t-1', 'fn')]
    reconcileTraceVisibility(
      verdicts,
      [bar({ id: 's1', groupKey: 'fn' })],
      rows,
      sel,
    )
    // Feed pruned the trace's spans; the listed row must stay hidden.
    expect(ids(reconcileTraceVisibility(verdicts, [], rows, sel))).toEqual([])
  })

  it('flips a live trace visible when a surviving span arrives', () => {
    const verdicts = new Map<string, boolean>()
    const sel = selection({ hiddenGroups: new Set(['harness::turn']) })
    const rows = [row('t-1', 'harness::turn')]
    const dispatch = bar({ id: 'root', groupKey: 'harness::turn' })
    expect(
      ids(reconcileTraceVisibility(verdicts, [dispatch], rows, sel)),
    ).toEqual([])
    const step = bar({ id: 'step', groupKey: 'harness::turn step' })
    expect(
      ids(reconcileTraceVisibility(verdicts, [dispatch, step], rows, sel)),
    ).toEqual(['t-1'])
  })

  it('drops cached verdicts once the trace leaves the list too', () => {
    const verdicts = new Map<string, boolean>()
    const sel = selection({ hiddenGroups: new Set(['fn']) })
    reconcileTraceVisibility(
      verdicts,
      [bar({ id: 's1', groupKey: 'fn' })],
      [row('t-1', 'fn')],
      sel,
    )
    expect(verdicts.has('t-1')).toBe(true)
    reconcileTraceVisibility(verdicts, [], [row('t-2', 'fn')], sel)
    expect(verdicts.has('t-1')).toBe(false)
  })

  it('returns the rows array unchanged when nothing hides', () => {
    const rows = [row('t-1'), row('t-2')]
    const kept = reconcileTraceVisibility(
      new Map(),
      [bar({ id: 's1', groupKey: 'fn' })],
      rows,
      selection(),
    )
    expect(kept).toBe(rows)
  })
})

describe('mergeFetchedVerdicts', () => {
  const sel = selection({ hiddenGroups: new Set(['fn']) })

  it('judges each requested trace by its fetched spans', () => {
    const verdicts = new Map<string, boolean>()
    mergeFetchedVerdicts(
      verdicts,
      ['t-hidden', 't-mixed', 't-gone'],
      [
        stored({
          span_id: 'h1',
          trace_id: 't-hidden',
          attributes: [['function_id', 'fn']],
        }),
        stored({
          span_id: 'm1',
          trace_id: 't-mixed',
          attributes: [['function_id', 'fn']],
        }),
        stored({
          span_id: 'm2',
          trace_id: 't-mixed',
          name: 'llm.completion',
        }),
      ],
      sel,
    )
    expect(verdicts.get('t-hidden')).toBe(false)
    expect(verdicts.get('t-mixed')).toBe(true)
    // No stored spans left → the detail would be empty → hidden.
    expect(verdicts.get('t-gone')).toBe(false)
  })

  it('does not let engine builtins keep a row alive — the detail view never shows them', () => {
    const verdicts = new Map<string, boolean>()
    mergeFetchedVerdicts(
      verdicts,
      ['t-1'],
      [
        stored({
          span_id: 'h1',
          attributes: [['function_id', 'fn']],
        }),
        stored({
          span_id: 'b1',
          name: 'call state::get',
          parent_span_id: 'h1',
          attributes: [
            ['function_id', 'state::get'],
            ['iii.function.kind', 'internal'],
          ],
        }),
      ],
      sel,
    )
    expect(verdicts.get('t-1')).toBe(false)
  })

  it('only trusts positive verdicts from a truncated response', () => {
    const verdicts = new Map<string, boolean>()
    const spans = [
      stored({
        span_id: 'h1',
        trace_id: 't-hidden',
        attributes: [['function_id', 'fn']],
      }),
      stored({ span_id: 'v1', trace_id: 't-visible', name: 'llm.completion' }),
    ]
    mergeFetchedVerdicts(verdicts, ['t-hidden', 't-visible'], spans, sel, 2)
    expect(verdicts.get('t-visible')).toBe(true)
    expect(verdicts.has('t-hidden')).toBe(false)
  })
})
