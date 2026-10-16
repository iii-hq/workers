import { describe, expect, it } from 'vitest'
import {
  cacheHitPct,
  conversationTree,
  filterGroups,
  type MetricsResponse,
  type SessionUsage,
  sortGroups,
  subAgentGroups,
  sumUsage,
  treeTotals,
} from './metrics'

function usage(
  session_id: string,
  parent_session_id: string | undefined,
  depth: number,
  extra: Partial<SessionUsage> = {},
): SessionUsage {
  return {
    session_id,
    parent_session_id,
    depth,
    turns: 1,
    function_calls: 2,
    function_call_errors: 0,
    input_tokens: 10,
    output_tokens: 20,
    cache_read_tokens: 300,
    cache_write_tokens: 40,
    cost_usd: 1,
    ...extra,
  }
}

/** The shape of a live deepwork run: the console chat is the root, each
    deepwork orchestrator a child, its workers grandchildren. */
const TREE: MetricsResponse = {
  root_session_id: 'console-root',
  complete: false,
  totals: { sessions: 6, turns: 6, function_calls: 12, function_call_errors: 0 },
  by_session: [
    usage('console-root', undefined, 0),
    usage('kan129-deepwork', 'console-root', 1),
    usage('kan176-deepwork', 'console-root', 1),
    usage('kan129-thinker', 'kan129-deepwork', 2),
    usage('kan176-implementer', 'kan176-deepwork', 2, { turns: 14, cost_usd: 1.19 }),
    usage('kan178-author', 'kan176-deepwork', 2, { function_call_errors: 1 }),
  ],
}

describe('conversationTree', () => {
  it('cuts the subtree of a conversation that is itself a child', () => {
    const tree = conversationTree(TREE, 'kan176-deepwork')
    expect(tree?.self.session_id).toBe('kan176-deepwork')
    expect(tree?.subAgents.map((row) => row.usage.session_id)).toEqual([
      'kan176-implementer',
      'kan178-author',
    ])
    // The sibling orchestrator and its worker are not this conversation's.
    const total = tree ? treeTotals(tree).total : null
    expect(total?.turns).toBe(1 + 14 + 1)
    expect(total?.function_call_errors).toBe(1)
    expect(total?.cost_usd).toBeCloseTo(3.19)
    expect(total?.partial).toEqual([])
  })

  it('walks depth-first with depth relative to the conversation', () => {
    const tree = conversationTree(TREE, 'console-root')
    expect(
      tree?.subAgents.map((row) => [row.usage.session_id, row.depth]),
    ).toEqual([
      ['kan129-deepwork', 1],
      ['kan129-thinker', 2],
      ['kan176-deepwork', 1],
      ['kan176-implementer', 2],
      ['kan178-author', 2],
    ])
    expect(tree && treeTotals(tree).total?.turns).toBe(19)
  })

  it('has no aggregate for a conversation without sub-agents', () => {
    const tree = conversationTree(TREE, 'kan176-implementer')
    expect(tree?.subAgents).toEqual([])
    expect(tree && treeTotals(tree).total).toBeNull()
    expect(tree?.self.parent_session_id).toBe('kan176-deepwork')
  })

  it('is null when the response does not carry the session', () => {
    expect(conversationTree(TREE, 'elsewhere')).toBeNull()
  })

  it('survives a cycle in parent links', () => {
    const cyclic: MetricsResponse = {
      ...TREE,
      by_session: [usage('a', 'b', 0), usage('b', 'a', 1)],
    }
    expect(conversationTree(cyclic, 'a')?.subAgents).toHaveLength(1)
  })
})

describe('sumUsage', () => {
  it('sums what was reported and marks a counter a session left out', () => {
    const sum = sumUsage([
      usage('a', undefined, 0, { cost_usd: undefined, input_tokens: 5 }),
      usage('b', 'a', 1, { cost_usd: 0.4, input_tokens: 7 }),
    ])
    // Two unpriced sub-agents out of 175 must not blank the total: it is a
    // lower bound, and says who is missing.
    expect(sum.cost_usd).toBe(0.4)
    expect(sum.partial).toEqual(['cost_usd'])
    expect(sum.unreported).toEqual(['a'])
    expect(sum.input_tokens).toBe(12)
  })

  it('takes the cache hit over sessions that reported both counts', () => {
    const sum = sumUsage([
      // Unpriced and unreported everything: not in the ratio.
      usage('gone', undefined, 0, {
        input_tokens: undefined,
        cache_read_tokens: undefined,
        cache_write_tokens: undefined,
        cost_usd: undefined,
      }),
      // A provider without cache reads would drag the ratio to 0: left out.
      usage('nocache', 'gone', 1, { input_tokens: 1000, cache_read_tokens: undefined }),
      // Cache reads without the fresh input they hit against: left out too.
      usage('noinput', 'gone', 1, { input_tokens: undefined, cache_read_tokens: 100 }),
      usage('a', 'gone', 1, { input_tokens: 10, cache_read_tokens: 90, cache_write_tokens: 0 }),
    ])
    expect(sum.cacheHit).toBe(90)
    expect(sum.partial).toContain('cache_read_tokens')
  })

  it('names only the sessions behind a partial counter', () => {
    // Nobody reports cache write (OpenAI, DeepSeek); only `c` lacks a cost.
    const sum = sumUsage([
      usage('root', undefined, 0, { cache_write_tokens: undefined }),
      usage('a', 'root', 1, { cache_write_tokens: undefined }),
      usage('c', 'root', 1, { cache_write_tokens: undefined, cost_usd: undefined }),
    ])
    expect(sum.partial).toEqual(['cost_usd'])
    expect(sum.unreported).toEqual(['c'])
    expect(sum.cache_write_tokens).toBeUndefined()
  })

  it('leaves a counter nobody reported absent, not a partial zero', () => {
    const sum = sumUsage([
      usage('a', undefined, 0, { cost_usd: undefined }),
      usage('b', 'a', 1, { cost_usd: undefined }),
    ])
    expect(sum.cost_usd).toBeUndefined()
    expect(sum.partial).toEqual([])
  })

  it('ignores a session that has no steps yet', () => {
    const sum = sumUsage([
      usage('a', undefined, 0, { cost_usd: 1 }),
      usage('b', 'a', 1, { turns: 0, cost_usd: undefined, input_tokens: undefined }),
    ])
    expect(sum.cost_usd).toBe(1)
    expect(sum.partial).toEqual([])
    expect(sum.unreported).toEqual([])
  })
})

describe('treeTotals', () => {
  it('takes this conversation\'s cost from the live snapshot', () => {
    const tree = conversationTree(TREE, 'kan176-deepwork')
    const { self, total } = tree ? treeTotals(tree, 2) : { self: null, total: null }
    expect(self?.cost_usd).toBe(2)
    expect(total?.cost_usd).toBeCloseTo(2 + 1.19 + 1)
  })
})

describe('sub-agent groups', () => {
  const tree = conversationTree(TREE, 'console-root')
  const groups = tree ? subAgentGroups(tree) : []

  it('groups each direct child with what it spawned', () => {
    expect(groups.map((g) => [g.head.session_id, g.rows.length])).toEqual([
      ['kan129-deepwork', 2],
      ['kan176-deepwork', 3],
    ])
    expect(groups[1].totals.cost_usd).toBeCloseTo(3.19)
  })

  it('sorts by subtree cost, by failures, and by the newest snapshot', () => {
    expect(sortGroups(groups, 'cost')[0].head.session_id).toBe('kan176-deepwork')
    expect(sortGroups(groups, 'failed')[0].totals.function_call_errors).toBe(1)
    const stamped = groups.map((g, i) => ({ ...g, lastActive: i === 0 ? 2 : 1 }))
    expect(sortGroups(stamped, 'recent')[0].head.session_id).toBe('kan129-deepwork')
    // A copy: the input order is untouched.
    expect(groups[0].head.session_id).toBe('kan129-deepwork')
  })

  it('filters by id, keeping the head of a group matched by a child', () => {
    expect(filterGroups(groups, '  ')).toBe(groups)
    const filtered = filterGroups(groups, 'IMPLEMENTER')
    expect(filtered.map((g) => g.rows.map((r) => r.session_id))).toEqual([
      ['kan176-deepwork', 'kan176-implementer'],
    ])
    // The group still knows its full size and totals.
    expect(filtered[0].size).toBe(2)
    expect(filtered[0].totals.cost_usd).toBeCloseTo(3.19)
    expect(filterGroups(groups, 'kan129-deep')[0].rows).toHaveLength(2)
    expect(filterGroups(groups, 'nothing')).toEqual([])
  })
})

describe('cacheHitPct', () => {
  it('is the cached share of the whole prompt', () => {
    expect(
      cacheHitPct({
        turns: 0,
        function_calls: 0,
        function_call_errors: 0,
        input_tokens: 7344,
        cache_read_tokens: 1_686_891,
        cache_write_tokens: 85_765,
      }),
    ).toBe(95)
  })

  it('is null when the fresh input count is unknown', () => {
    expect(
      cacheHitPct({ turns: 1, function_calls: 0, function_call_errors: 0, cache_read_tokens: 900 }),
    ).toBeNull()
  })

  it('is null without prompt tokens', () => {
    expect(
      cacheHitPct({ turns: 0, function_calls: 0, function_call_errors: 0 }),
    ).toBeNull()
  })
})
