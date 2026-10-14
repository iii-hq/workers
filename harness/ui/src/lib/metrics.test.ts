import { describe, expect, it } from 'vitest'
import {
  cacheHitPct,
  conversationTree,
  type MetricsResponse,
  type SessionUsage,
  sumUsage,
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

/** The shape of a live ultracode run: the console chat is the root, each
    ultracode orchestrator a child, its workers grandchildren. */
const TREE: MetricsResponse = {
  root_session_id: 'console-root',
  complete: false,
  totals: { sessions: 6, turns: 6, function_calls: 12, function_call_errors: 0 },
  by_session: [
    usage('console-root', undefined, 0),
    usage('kan129-ultracode', 'console-root', 1),
    usage('kan176-ultracode', 'console-root', 1),
    usage('kan129-thinker', 'kan129-ultracode', 2),
    usage('kan176-implementer', 'kan176-ultracode', 2, { turns: 14, cost_usd: 1.19 }),
    usage('kan178-author', 'kan176-ultracode', 2, { function_call_errors: 1 }),
  ],
}

describe('conversationTree', () => {
  it('cuts the subtree of a conversation that is itself a child', () => {
    const tree = conversationTree(TREE, 'kan176-ultracode')
    expect(tree?.self.session_id).toBe('kan176-ultracode')
    expect(tree?.subAgents.map((row) => row.usage.session_id)).toEqual([
      'kan176-implementer',
      'kan178-author',
    ])
    // The sibling orchestrator and its worker are not this conversation's.
    expect(tree?.withSubAgents?.turns).toBe(1 + 14 + 1)
    expect(tree?.withSubAgents?.function_call_errors).toBe(1)
    expect(tree?.withSubAgents?.cost_usd).toBeCloseTo(3.19)
  })

  it('walks depth-first with depth relative to the conversation', () => {
    const tree = conversationTree(TREE, 'console-root')
    expect(
      tree?.subAgents.map((row) => [row.usage.session_id, row.depth]),
    ).toEqual([
      ['kan129-ultracode', 1],
      ['kan129-thinker', 2],
      ['kan176-ultracode', 1],
      ['kan176-implementer', 2],
      ['kan178-author', 2],
    ])
    expect(tree?.withSubAgents?.turns).toBe(19)
  })

  it('has no aggregate for a conversation without sub-agents', () => {
    const tree = conversationTree(TREE, 'kan176-implementer')
    expect(tree?.subAgents).toEqual([])
    expect(tree?.withSubAgents).toBeNull()
    expect(tree?.self.parent_session_id).toBe('kan176-ultracode')
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
  it('makes a counter unknown when a session with steps did not report it', () => {
    const sum = sumUsage([
      usage('a', undefined, 0, { cost_usd: undefined, input_tokens: 5 }),
      usage('b', 'a', 1, { cost_usd: 0.4, input_tokens: 7 }),
    ])
    // $0.40 would be a partial total shown as the whole.
    expect(sum.cost_usd).toBeUndefined()
    expect(sum.input_tokens).toBe(12)
  })

  it('ignores a session that has no steps yet', () => {
    const sum = sumUsage([
      usage('a', undefined, 0, { cost_usd: 1 }),
      usage('b', 'a', 1, { turns: 0, cost_usd: undefined, input_tokens: undefined }),
    ])
    expect(sum.cost_usd).toBe(1)
    expect(sum.input_tokens).toBe(10)
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
