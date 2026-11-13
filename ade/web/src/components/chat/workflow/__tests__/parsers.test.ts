import { readFileSync } from 'node:fs'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { FunctionCallMessage } from '@/types/chat'
import { WorkflowToolView } from '../index'
import {
  consumedDeps,
  inputSources,
  isJoinNode,
  isWorkflowFunction,
  nodeDefSchema,
  safeParseRequest,
  safeParseResponse,
  startRequestSchema,
  statusResponseSchema,
  tallyNodes,
  WORKFLOW_FUNCTION_IDS,
  type WorkflowDef,
} from '../parsers'
import { DagSummary } from '../shared'

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

  it('does not throw on a source node that omits input', () => {
    expect(isJoinNode(nodeDefSchema.parse({ agent: { model: 'm' } }))).toBe(
      false,
    )
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

interface FlatStartCase {
  name: string
  request: unknown
  stored: WorkflowDef
}

// The worker's lower() test reads the same file, so the two lifts cannot drift.
const { cases } = JSON.parse(
  readFileSync(
    new URL(
      '../../../../../../../workflow/tests/fixtures/flat_start_cases.json',
      import.meta.url,
    ),
    'utf8',
  ),
) as { cases: FlatStartCase[] }

const definitionOf = (request: unknown) => {
  const def = safeParseRequest(startRequestSchema, request)?.definition
  if (!def) throw new Error('start request does not parse')
  return def
}

describe('startRequestSchema', () => {
  it.each(cases)(
    'lifts $name as the worker stores it',
    ({ request, stored }) => {
      const def = definitionOf(request)
      expect(def.output?.from).toEqual(stored.output?.from)
      expect(Object.keys(def.nodes).sort()).toEqual(
        Object.keys(stored.nodes).sort(),
      )
      const wiring = (nodes: WorkflowDef['nodes']) =>
        Object.entries(nodes).map(([id, node]) => ({
          id,
          input: node.input,
          depends_on: node.depends_on,
          fanout: node.fanout ?? undefined,
        }))
      expect(wiring(def.nodes)).toEqual(
        expect.arrayContaining(wiring(stored.nodes)),
      )
    },
  )

  it('lifts a fanout array without reads, and a fanout path that reads another node', () => {
    const def = definitionOf({
      nodes: {
        critic: { task: 't', schema: { type: 'object' } },
        draft: { task: 't', model: null },
        angles: { task: 't', fanout: ['security', 'cost'], reads: null },
        refute: { task: 't', fanout: 'critic.findings', reads: 'draft' },
      },
      output: null,
    })
    expect(def.nodes.angles).toMatchObject({
      input: { from: 'fanout_item' },
      depends_on: [],
      fanout: { items: ['security', 'cost'] },
    })
    expect(def.nodes.refute).toMatchObject({
      input: { from: 'node:draft' },
      depends_on: ['draft', 'critic'],
      fanout: { over: 'node:critic.findings' },
    })
    expect(def.output?.from).toEqual(['node:angles', 'node:refute'])
  })

  it('does not parse an old-shape request', () => {
    expect(
      safeParseRequest(startRequestSchema, {
        definition: {
          version: 1,
          nodes: {
            a: { agent: { model: 'm' }, input: { from: 'run_input' } },
          },
          output: { from: 'node:a' },
        },
        input: {},
      }),
    ).toBeNull()
  })

  it('does not count a self-read as read, as lower() does', () => {
    expect(
      definitionOf({ nodes: { a: { task: 't', reads: 'a' } } }).output?.from,
    ).toBe('node:a')
  })

  it('reads a blank model as omitted, as lower() does', () => {
    expect(
      definitionOf({ nodes: { a: { task: 't', model: '  ' } } }).nodes.a.agent
        ?.model,
    ).toBeUndefined()
  })

  it('parses a node without a task so the view never blanks', () => {
    expect(definitionOf({ nodes: { gen: {} } }).output?.from).toBe('node:gen')
  })

  it('keeps node keys it does not lift, and reads loop pass uids', () => {
    const loop = {
      body: ['work'],
      while: { field: 'verdict', equals: 'reject' },
      max_repeats: 2,
    }
    const def = definitionOf({
      nodes: {
        work: { task: 't', profile: 'deepwork-impl' },
        verify: {
          task: 't',
          model: 'm',
          schema: { type: 'object' },
          reads: 'work',
          loop,
        },
      },
    })
    expect(def.nodes.verify.loop).toEqual(loop)
    expect(def.nodes.verify.agent.model).toBe('m')
    expect(def.output?.from).toBe('node:verify')

    const status = statusResponseSchema.parse({
      status: 'awaiting_nodes',
      nodes: {
        work: 'done',
        verify: 'done',
        'work#it1': 'running',
        'verify#it1': 'pending',
      },
    })
    expect(Object.keys(status.nodes)).toEqual([
      'work',
      'verify',
      'work#it1',
      'verify#it1',
    ])
    expect(tallyNodes(status.nodes)).toMatchObject({
      total: 4,
      done: 2,
      running: 1,
      pending: 1,
    })
  })
})

describe('WorkflowToolView.tryRender', () => {
  const startCall = (input: unknown) =>
    ({
      id: 'm',
      role: 'function-call',
      functionId: 'workflow::start',
      input,
      output: { run_id: 'r_1' },
      createdAt: 1,
    }) as FunctionCallMessage

  it('draws a flat start, and leaves an unparseable one to the default panes', () => {
    expect(
      WorkflowToolView.tryRender(startCall({ nodes: { a: { task: 't' } } })),
    ).not.toBeNull()
    expect(
      WorkflowToolView.tryRender(
        startCall({ definition: { nodes: { a: { agent: {} } } } }),
      ),
    ).toBeNull()
  })
})

describe('DagSummary', () => {
  it('counts a fanout source as read and sizes a written fanout array', () => {
    const html = renderToStaticMarkup(
      createElement(DagSummary, {
        def: definitionOf({
          nodes: {
            plan: { task: 't', schema: { type: 'object' } },
            draft: { task: 't' },
            critic: { task: 't', fanout: ['security', 'cost'], reads: 'draft' },
            refute: { task: 't', fanout: 'plan.items', reads: 'draft' },
          },
        }),
      }),
    )
    expect(html).toContain('⋔ over 2 items')
    expect(html).toContain('⋔ over plan.items')
  })
})

describe('statusResponseSchema', () => {
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
