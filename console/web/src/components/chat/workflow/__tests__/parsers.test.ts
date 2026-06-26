import { describe, expect, it } from 'vitest'
import {
  consumedDeps,
  inputSources,
  isJoinNode,
  isWorkflowFunction,
  nodeDefSchema,
  safeParseResponse,
  startRequestSchema,
  statusResponseSchema,
  tallyNodes,
  WORKFLOW_FUNCTION_IDS,
} from '../parsers'

describe('isWorkflowFunction', () => {
  it('matches the known set and rejects others', () => {
    for (const id of WORKFLOW_FUNCTION_IDS)
      expect(isWorkflowFunction(id)).toBe(true)
    expect(isWorkflowFunction('shell::exec')).toBe(false)
    expect(isWorkflowFunction('workflow::nope')).toBe(false)
  })
})

describe('inputSources / consumedDeps', () => {
  it('normalises a single string source', () => {
    expect(inputSources('node:a')).toEqual(['node:a'])
    expect(inputSources(undefined)).toEqual([])
    expect(inputSources('')).toEqual([])
  })

  it('keeps array sources (the join form)', () => {
    expect(inputSources(['node:a', 'node:b'])).toEqual(['node:a', 'node:b'])
  })

  it('extracts node dep ids and ignores run_input/fanout_item', () => {
    expect(consumedDeps(['node:a', 'node:b.result.x', 'run_input'])).toEqual([
      'a',
      'b',
    ])
    expect(consumedDeps('fanout_item')).toEqual([])
  })
})

describe('isJoinNode', () => {
  const node = (input: unknown, depends_on: string[]) =>
    nodeDefSchema.parse({
      agent: { model: 'm' },
      input: { from: input },
      depends_on,
    })

  it('flags a node reading more than one upstream', () => {
    expect(isJoinNode(node(['node:a', 'node:b'], ['a', 'b']))).toBe(true)
  })

  it('flags a node depending on more than one node', () => {
    // The footgun shape: depends_on two, reads one. Still a join to highlight.
    expect(isJoinNode(node('node:a', ['a', 'b']))).toBe(true)
  })

  it('does not flag a plain single-dep node', () => {
    expect(isJoinNode(node('node:a', ['a']))).toBe(false)
    expect(isJoinNode(node('run_input', []))).toBe(false)
  })
})

describe('tallyNodes', () => {
  it('counts each node + fanned item by state', () => {
    const counts = tallyNodes({
      a: 'done',
      b: 'done',
      'c#0': 'running',
      'c#1': 'pending',
      d: 'failed',
    })
    expect(counts).toEqual({
      total: 5,
      done: 2,
      running: 1,
      pending: 1,
      failed: 1,
      cancelled: 0,
    })
  })
})

describe('schema parsing', () => {
  it('parses a start request with an array (Many) input.from', () => {
    const parsed = startRequestSchema.safeParse({
      definition: {
        version: 1,
        nodes: {
          a: { agent: { model: 'm' }, input: { from: 'run_input' } },
          j: {
            agent: { model: 'm' },
            input: { from: ['node:a'] },
            depends_on: ['a'],
          },
        },
        output: { from: 'node:j' },
      },
      input: {},
    })
    expect(parsed.success).toBe(true)
  })

  it('unwraps a harness-enveloped status response', () => {
    const enveloped = {
      content: [{ type: 'text', text: '{}' }],
      details: { status: 'completed', nodes: { a: 'done' } },
      terminate: true,
    }
    const resp = safeParseResponse(statusResponseSchema, enveloped)
    expect(resp?.status).toBe('completed')
    expect(tallyNodes(resp?.nodes ?? {}).done).toBe(1)
  })
})
