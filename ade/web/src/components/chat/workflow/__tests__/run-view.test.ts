import { describe, expect, it } from 'vitest'
import type { Conversation, Message } from '@/types/chat'
import {
  buildRunView,
  collectWorkflowRuns,
  labelTitle,
  nodeDepths,
  nodeFacts,
  nodeLabel,
  nodeLine,
  nodeMark,
  parseRunDetails,
  runFacts,
  runMark,
  soleFacts,
  soleNode,
  splitNodeUid,
} from '../run-view'
import { BATCH, LIVE, T } from './run-fixtures'

const task = 't'
/** work ⇄ verify (a loop), then plan → map (a fanout) → critic → a_report. */
const START = {
  nodes: {
    work: { task },
    verify: { task, reads: 'work', loop: { body: ['work'] } },
    plan: { task, reads: 'verify' },
    map: { task, fanout: 'plan.items' },
    critic: { task, reads: ['map', 'verify'] },
    a_report: { task, reads: 'critic' },
  },
}

const parsed = (value: unknown) => {
  const status = parseRunDetails(value)
  if (!status) throw new Error('fixture does not parse')
  return status
}
const order = (view: ReturnType<typeof buildRunView>) =>
  view.phases.map(
    (phase) => `${phase.phase}: ${phase.nodes.map((node) => node.uid).join()}`,
  )
const start = (extra: Record<string, unknown>): Message =>
  ({
    id: 'm',
    role: 'function-call',
    functionId: 'workflow::start',
    createdAt: 1,
    ...extra,
  }) as unknown as Message
const session = (createdAt: number, sessionMetadata: object): Conversation =>
  ({ id: `s${createdAt}`, createdAt, sessionMetadata }) as Conversation

describe('parseRunDetails', () => {
  it('reads a status with details', () => {
    const status = parsed(BATCH)
    expect(status.status).toBe('completed')
    expect(status.node_details?.['batch-work'].usage?.input_tokens).toBe(52226)
  })

  it('unwraps a harness envelope', () => {
    expect(parseRunDetails({ content: [], details: BATCH })?.run_id).toBe(
      'r_3f03',
    )
  })

  it('is null for a run the worker no longer has, or a reply it cannot read', () => {
    expect(parseRunDetails(null)).toBeNull()
    expect(parseRunDetails(undefined)).toBeNull()
    expect(parseRunDetails({})).toBeNull()
    expect(
      parseRunDetails({ status: 'running', nodes: { a: 'exploded' } }),
    ).toBeNull()
  })
})

describe('labelTitle', () => {
  const of = (...labels: string[]) =>
    labelTitle(labels.map((label) => ({ label })))
  it('names a run with no start call after its node labels', () => {
    expect(of()).toBeUndefined()
    expect(of('KAN-5 critic', 'KAN-5 critic')).toBe('KAN-5 critic')
    expect(of('KAN-5 batch r1w1 tests', 'KAN-5 batch r1w1 check')).toBe(
      'KAN-5 batch r1w1',
    )
    // A shared start that is not a whole word names nothing.
    expect(
      of('KAN-6 · Implement slugify', 'KAN-7 · Implement count_vowels'),
    ).toBe('KAN-6 · Implement slugify +1')
  })
})

describe('splitNodeUid', () => {
  it('reads a loop pass, a fanout item and a plain node', () => {
    expect(splitNodeUid('v#it2')).toEqual({
      base: 'v',
      suffix: '#it2',
      index: 2,
    })
    expect(splitNodeUid('map#10')).toEqual({
      base: 'map',
      suffix: '#10',
      index: 10,
    })
    expect(splitNodeUid('edit')).toEqual({ base: 'edit', index: 0 })
    expect(splitNodeUid('v#itx')).toEqual({ base: 'v#itx', index: 0 })
  })
})

describe('nodeDepths', () => {
  it('counts the nodes each node waits behind', () => {
    expect(nodeDepths(parsedDef(START))).toEqual({
      work: 0,
      verify: 1,
      plan: 2,
      map: 3,
      critic: 4,
      a_report: 5,
    })
  })

  it('survives a cycle and a missing node', () => {
    const depths = nodeDepths(
      parsedDef({
        nodes: {
          a: { task, reads: 'b' },
          b: { task, reads: 'a' },
          c: { task, reads: 'gone' },
        },
      }),
    )
    expect(Object.keys(depths).sort()).toEqual(['a', 'b', 'c'])
    expect(depths.c).toBe(1)
  })
})

function parsedDef(request: unknown) {
  const ref = collectWorkflowRuns(
    [
      start({
        input: request,
        output: { content: [], details: { run_id: 'r_def' } },
      }),
    ],
    [],
  )[0]
  if (!ref.def) throw new Error('definition does not parse')
  return ref.def
}

describe('collectWorkflowRuns', () => {
  it('reads started runs from the transcript, oldest first', () => {
    const refs = collectWorkflowRuns(
      [
        start({
          createdAt: 50,
          description: 'Second',
          input: START,
          output: { content: [], details: { run_id: 'r_b' } },
        }),
        start({
          createdAt: 20,
          description: 'First',
          input: START,
          output: { run_id: 'r_a' },
        }),
        { id: 'x', role: 'assistant', content: 'r_zzz', createdAt: 5 },
      ] as Message[],
      [],
    )
    expect(refs.map((ref) => [ref.runId, ref.at, ref.title])).toEqual([
      ['r_a', 20, 'First'],
      ['r_b', 50, 'Second'],
    ])
    expect(Object.keys(refs[0].def?.nodes ?? {})).toHaveLength(6)
  })

  it('skips a start that is running, failed, or left out of the page', () => {
    expect(
      collectWorkflowRuns(
        [
          start({ input: START, running: true }),
          start({
            input: {},
            output: { error: { kind: 'function_error', message: 'bad' } },
          }),
          start({ input: undefined, unloaded: true }),
        ],
        [],
      ),
    ).toEqual([])
  })

  it('adds the runs node sessions name, once each, at their first session', () => {
    const refs = collectWorkflowRuns(
      [
        start({
          createdAt: 50,
          description: 'Known',
          output: { run_id: 'r_b' },
        }),
      ],
      [
        session(12, { workflow_run_id: 'r_a' }),
        session(11, { workflow_run_id: 'r_a' }),
        session(60, { workflow_run_id: 'r_b' }),
        session(70, { parent_session_id: 'main' }),
        session(80, { workflow_run_id: 7 }),
      ],
    )
    expect(refs.map((ref) => [ref.runId, ref.at, ref.title])).toEqual([
      ['r_a', 11, undefined],
      ['r_b', 50, 'Known'],
    ])
  })
})

describe('buildRunView', () => {
  it('groups a finished run by phase, in the order its nodes started', () => {
    const view = buildRunView(
      { runId: 'r_3f03', at: 1, title: 'Writing and checking tests' },
      parsed(BATCH),
    )
    expect(view.title).toBe('Writing and checking tests')
    expect(view.live).toBe(false)
    expect(order(view)).toEqual(['test: batch-work', 'check: batch-check'])
    expect(view.phases.map((phase) => [phase.done, phase.total])).toEqual([
      [1, 1],
      [1, 1],
    ])
    expect(view.unlisted).toBe(0)
    expect(runFacts(view)).toBe('test → check · 292k tokens · 1m 51s')
    const [work] = view.phases[0].nodes
    expect(work.sessionId).toBe('wf_r_3f03_batch-work@r0')
    expect(nodeFacts(work)).toBe('codex/gpt-6-luna · 256k tokens')
    expect(nodeMark(work, T)).toEqual({ tone: 'ok', text: '1m 25s' })
    expect(runMark(view, 1794408448575 + 7 * 60_000)).toEqual({
      tone: 'ok',
      text: '7m',
    })
  })

  it('keeps loop passes with their phase and waiting nodes in pipeline order', () => {
    const view = buildRunView(
      { runId: 'r_live', at: 1, def: parsedDef(START) },
      parsed(LIVE),
    )
    expect(order(view)).toEqual([
      'work: work,work#it1',
      'check: verify,verify#it1',
      'plan: plan',
      'map: map',
      'critic: critic',
      ': a_report',
    ])
    const [, pass] = view.phases[1].nodes
    expect([pass.label, pass.suffix]).toEqual(['Check', '#it1'])
    expect(view.phases[0].nodes[1].error).toBe(
      'no provider registered for model x',
    )
    // `phases` counts every pass; a node without a phase is counted here.
    expect(view.phases[0]).toMatchObject({ done: 1, total: 2 })
    expect(view.phases.at(-1)).toMatchObject({ phase: '', done: 0, total: 1 })
    expect(view.title).toBe('Work +5')
    expect(runFacts(view)).toBe(
      '2/8 done · 1 running · 1 failed · 1.5k tokens · $0.423',
    )
    expect(runMark(view, T)).toEqual({
      tone: 'accent',
      pulse: true,
      text: 'running',
    })
  })

  it('without the definition, lists waiting nodes by uid', () => {
    const view = buildRunView({ runId: 'r_live', at: 1 }, parsed(LIVE))
    expect(view.phases.map((phase) => phase.phase)).toEqual([
      'work',
      'check',
      '',
      'critic',
      'map',
      'plan',
    ])
  })

  it('orders fanout items by number and names a lone node run after it', () => {
    const nodes = Object.fromEntries(
      [10, 2, 1].map((i) => [`map#${i}`, { state: 'pending', label: 'Map' }]),
    )
    const view = buildRunView(
      { runId: 'r_fan', at: 1 },
      parsed({ status: 'running', nodes: {}, node_details: nodes }),
    )
    expect(order(view)).toEqual([': map#1,map#2,map#10'])
    const lone = buildRunView(
      { runId: 'r_one', at: 1 },
      parsed({
        status: 'completed',
        nodes: { a: 'done' },
        node_details: { a: { state: 'done', label: 'KAN-5 baseline' } },
      }),
    )
    expect(lone.title).toBe('KAN-5 baseline')
  })

  it('counts the nodes a status leaves without details', () => {
    const view = buildRunView(
      { runId: 'r_x', at: 1 },
      parsed({
        status: 'failed',
        nodes: { a: 'done', b: 'failed' },
        result_error: 'node b failed',
      }),
    )
    expect([view.phases.length, view.unlisted, view.error]).toEqual([
      0,
      2,
      'node b failed',
    ])
    expect(runMark(view, T)).toEqual({ tone: 'alert', text: 'failed' })
  })

  it('marks totals over part of the nodes, and opens only sessions that were sent', () => {
    const view = buildRunView(
      { runId: 'r_x', at: 1 },
      parsed({
        status: 'running',
        nodes: { a: 'done', b: 'pending', c: 'done' },
        node_details: {
          a: {
            state: 'done',
            session_id: 'wf_r_x_a@r0',
            started_at: 1,
            usage: { input_tokens: 1500 },
          },
          b: { state: 'pending', session_id: 'wf_r_x_b@r0' },
        },
      }),
    )
    const [a, b] = view.phases.flatMap((phase) => phase.nodes)
    expect([a.sessionId, b.sessionId]).toEqual(['wf_r_x_a@r0', undefined])
    expect(runFacts(view)).toBe('2/3 done · ≥1.5k tokens')
  })
})

describe('nodeMark', () => {
  const view = buildRunView({ runId: 'r_live', at: 1 }, parsed(LIVE))
  const nodes = view.phases.flatMap((phase) => phase.nodes)
  const node = (uid: string) => {
    const found = nodes.find((candidate) => candidate.uid === uid)
    if (!found) throw new Error(uid)
    return found
  }

  it('shows how long a running node has run, then how long it has been idle', () => {
    const pass = node('verify#it1')
    expect(nodeMark(pass, T + 6000 + 20_000)).toEqual({
      tone: 'accent',
      pulse: true,
      text: '20s',
    })
    expect(nodeMark(pass, T + 7000 + 180_000)).toEqual({
      tone: 'warn',
      text: 'idle 3m',
    })
    expect(nodeMark({ ...pass, parked: true }, T + 7000 + 180_000)).toEqual({
      tone: 'ink',
      text: 'parked',
    })
    expect(
      nodeMark({ ...pass, awaitingFunctions: true }, T + 7000 + 180_000),
    ).toMatchObject({ tone: 'accent', pulse: true })
  })

  it('names the other states', () => {
    expect(nodeMark(node('work'), T)).toEqual({ tone: 'ok', text: '2.0s' })
    expect(nodeMark(node('work#it1'), T)).toEqual({
      tone: 'alert',
      text: 'failed',
    })
    expect(nodeMark(node('plan'), T)).toEqual({
      tone: 'ghost',
      text: 'pending',
    })
    expect(nodeMark({ ...node('plan'), state: 'cancelled' }, T)).toEqual({
      tone: 'warn',
      text: 'cancelled',
    })
  })

  it('lists the facts a node has', () => {
    expect(nodeFacts(node('verify#it1'))).toBe('1.5k tokens · $0.423')
    // The profile is the row's tooltip, not its line.
    expect(nodeFacts(node('plan'))).toBe('')
    expect(nodeFacts(node('map'))).toBe('')
  })
})

describe('compact rows', () => {
  const batch = () =>
    buildRunView(
      { runId: 'r_3f03', at: 1, title: 'KAN-5 batch r1w1' },
      parseRunDetails(BATCH) as NonNullable<ReturnType<typeof parseRunDetails>>,
    )

  it('names a node after what its run name leaves, and its phase only when the label does not', () => {
    const view = batch()
    const [work, check] = view.phases.flatMap((phase) => phase.nodes)
    expect(nodeLabel(view, work)).toBe('tests')
    expect(nodeLabel(view, check)).toBe('check')
    expect(nodeLine(view, work)).toBe('codex/gpt-6-luna · 256k tokens')
    const titled = { ...view, title: 'Writing and checking tests' }
    expect(nodeLabel(titled, work)).toBe('KAN-5 batch r1w1 tests')
    expect(nodeLine(titled, work)).toBe('test · codex/gpt-6-luna · 256k tokens')
  })

  it('reads a run of one node as that node', () => {
    const view = batch()
    expect(soleNode(view)).toBeNull()
    const [work] = view.phases.flatMap((phase) => phase.nodes)
    const one = {
      ...view,
      nodeCount: 1,
      phases: [{ ...view.phases[0], nodes: [work] }],
    }
    expect(soleNode(one)).toBe(work)
    expect(soleFacts(one, work)).toBe('codex/gpt-6-luna · 256k tokens · 1m 51s')
    expect(soleNode({ ...one, unlisted: 1 })).toBeNull()
  })
})
