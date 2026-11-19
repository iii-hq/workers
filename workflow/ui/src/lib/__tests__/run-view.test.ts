import { describe, expect, it } from 'vitest'
import {
  buildRunView,
  groupsByPhase,
  labelTitle,
  nodeFacts,
  nodeLabel,
  nodeLine,
  nodeMark,
  parseRunDetails,
  parseRunList,
  phaseCurrent,
  phaseFacts,
  phaseMark,
  phaseNodeLine,
  phaseOpens,
  phaseState,
  runFacts,
  runMark,
  soleFacts,
  soleNode,
  solePhaseNode,
  splitNodeUid,
  underPhase,
} from '../run-view'
import { BATCH, KIT, KIT_NEXT, LIVE, T } from './run-fixtures'

const parsed = (value: unknown) => {
  const status = parseRunDetails(value)
  if (!status) throw new Error('fixture does not parse')
  return status
}
const order = (view: ReturnType<typeof buildRunView>) =>
  view.phases.map((phase) => `${phase.phase}: ${phase.nodes.map((node) => node.uid).join()}`)

describe('parseRunDetails', () => {
  it('reads a status with details', () => {
    const status = parsed(BATCH)
    expect(status.status).toBe('completed')
    expect(status.node_details?.['batch-work'].usage?.input_tokens).toBe(52226)
  })

  it('unwraps a harness envelope', () => {
    expect(parseRunDetails({ content: [], details: BATCH })?.run_id).toBe('r_3f03')
  })

  it('is null for a run the worker no longer has, or a reply it cannot read', () => {
    expect(parseRunDetails(null)).toBeNull()
    expect(parseRunDetails(undefined)).toBeNull()
    expect(parseRunDetails({})).toBeNull()
    expect(parseRunDetails({ status: 'running', nodes: { a: 'exploded' } })).toBeNull()
  })
})

describe('labelTitle', () => {
  const of = (...labels: string[]) => labelTitle(labels.map((label) => ({ label })))
  it('names a run with no start call after its node labels', () => {
    expect(of()).toBeUndefined()
    expect(of('KAN-5 critic', 'KAN-5 critic')).toBe('KAN-5 critic')
    expect(of('KAN-5 batch r1w1 tests', 'KAN-5 batch r1w1 check')).toBe('KAN-5 batch r1w1')
    // A shared start that is not a whole word names nothing.
    expect(of('KAN-6 · Implement slugify', 'KAN-7 · Implement count_vowels')).toBe('KAN-6 · Implement slugify +1')
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

describe('parseRunList', () => {
  it('reads the runs a chat started, in the order the worker lists them', () => {
    const reply = {
      runs: [
        { run_id: 'r_new', status: 'running', created_at: 20, updated_at: 30 },
        { run_id: 'r_old', status: 'completed', created_at: 10, updated_at: 12 },
      ],
    }
    const refs = [
      { runId: 'r_new', at: 20 },
      { runId: 'r_old', at: 10 },
    ]
    expect(parseRunList(reply)).toEqual(refs)
    expect(parseRunList({ content: [], details: reply })).toEqual(refs)
    expect(parseRunList({ runs: [] })).toEqual([])
  })

  it('keeps the title a run has', () => {
    const refs = parseRunList({
      runs: [
        { run_id: 'r_a', status: 'running', title: 'Writing and checking tests', created_at: 20, updated_at: 30 },
        { run_id: 'r_b', status: 'running', title: null, created_at: 10, updated_at: 12 },
      ],
    })
    expect(refs?.map((ref) => [ref.runId, ref.title])).toEqual([
      ['r_a', 'Writing and checking tests'],
      ['r_b', undefined],
    ])
  })

  it('is null for a reply it cannot read', () => {
    expect(parseRunList(null)).toBeNull()
    expect(parseRunList({})).toBeNull()
    expect(parseRunList({ runs: [{ status: 'running' }] })).toBeNull()
  })
})

describe('buildRunView', () => {
  it('groups a finished run by phase, in the order its nodes started', () => {
    const view = buildRunView({ runId: 'r_3f03', at: 1, title: 'Writing and checking tests' }, parsed(BATCH))
    expect(view.title).toBe('Writing and checking tests')
    // Without a title, named after what its node labels share.
    expect(buildRunView({ runId: 'r_3f03', at: 1 }, parsed(BATCH)).title).toBe('KAN-5 batch r1w1')
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
    const view = buildRunView({ runId: 'r_live', at: 1 }, parsed(LIVE))
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
    expect(view.phases[0].nodes[1].error).toBe('no provider registered for model x')
    // `phases` counts every pass; a node without a phase is counted here.
    expect(view.phases[0]).toMatchObject({ done: 1, total: 2 })
    expect(view.phases.at(-1)).toMatchObject({ phase: '', done: 0, total: 1 })
    expect(view.title).toBe('Work +5')
    expect(runFacts(view)).toBe(
      // verify and work#it1 started, but their usage is missing.
      '2/8 done · 1 running · 1 failed · ≥1.5k tokens · ≥$0.423',
    )
    expect(runMark(view, T)).toEqual({
      tone: 'accent',
      pulse: true,
      text: 'running',
    })
  })

  it('without depths, lists waiting nodes by uid', () => {
    const view = buildRunView(
      { runId: 'r_live', at: 1 },
      parsed({
        ...LIVE,
        node_details: Object.fromEntries(
          Object.entries(LIVE.node_details).map(([uid, { depth: _, ...detail }]) => [uid, detail]),
        ),
      }),
    )
    expect(view.phases.map((phase) => phase.phase)).toEqual(['work', 'check', '', 'critic', 'map', 'plan'])
  })

  it('orders fanout items by number and names a run after its node, else its id', () => {
    const nodes = Object.fromEntries([10, 2, 1].map((i) => [`map#${i}`, { state: 'pending', label: 'Map' }]))
    const view = buildRunView({ runId: 'r_fan', at: 1 }, parsed({ status: 'running', nodes: {}, node_details: nodes }))
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
    expect(view.title).toBe('Map')
    const unnamed = buildRunView(
      { runId: 'r_0123456789abcdef', at: 1 },
      parsed({ status: 'running', nodes: { a: 'pending' } }),
    )
    expect(unnamed.title).toBe('Run r_01234567')
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
    expect([view.phases.length, view.unlisted, view.error]).toEqual([0, 2, 'node b failed'])
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

  it('marks totals that miss a started node whose usage read failed', () => {
    const view = buildRunView(
      { runId: 'r_x', at: 1 },
      parsed({
        status: 'completed',
        nodes: { a: 'done', b: 'done' },
        node_details: {
          a: {
            state: 'done',
            started_at: 1,
            usage: { input_tokens: 1500, cost_usd: 0.5 },
          },
          b: { state: 'done', started_at: 2 },
        },
      }),
    )
    expect(runFacts(view)).toBe('2 nodes · ≥1.5k tokens · ≥$0.500')
  })

  describe('a node whose transient retry left generations without usage', () => {
    const run = (usage: Record<string, unknown>) => {
      const view = buildRunView(
        { runId: 'r_e64a', at: 1 },
        parsed({
          status: 'completed',
          nodes: { plan: 'done', 'explore-a': 'done' },
          node_details: {
            plan: {
              state: 'done',
              started_at: 1,
              model: 'codex/gpt-6.1-terra',
              usage: { input_tokens: 16464, output_tokens: 900, turns: 1 },
            },
            'explore-a': {
              state: 'done',
              started_at: 2,
              model: 'codex/gpt-6.1-terra',
              usage: { complete: true, ...usage },
            },
          },
        }),
      )
      const [, explore] = view.phases.flatMap((phase) => phase.nodes)
      return { view, explore }
    }

    it('reads no known counter as an unknown count, never 0 tokens', () => {
      const { view, explore } = run({
        input_tokens: null,
        output_tokens: null,
        cache_read_tokens: null,
        cache_write_tokens: null,
        turns: 5,
      })
      expect(explore.tokens).toBeNull()
      expect(nodeFacts(explore)).toBe('codex/gpt-6.1-terra')
      expect(runFacts(view)).toBe('2 nodes · ≥17k tokens')
    })

    it('marks the tokens it did report as a lower bound', () => {
      const { view, explore } = run({
        input_tokens: 16464,
        output_tokens: 900,
        unreported_generations: 4,
        turns: 5,
      })
      expect(nodeFacts(explore)).toBe('codex/gpt-6.1-terra · ≥17k tokens')
      expect(runFacts(view)).toBe('2 nodes · ≥35k tokens')
    })

    it('counts a node that generated nothing as a measured zero', () => {
      const { view, explore } = run({ turns: 0, sessions: 1 })
      expect(explore.tokens).toBe(0)
      expect(runFacts(view)).toBe('2 nodes · 17k tokens')
    })

    it('reads a node whose session the metrics could not find as unknown', () => {
      const { view, explore } = run({ turns: 0, sessions: 0, complete: false })
      expect(explore.tokens).toBeNull()
      expect(nodeFacts(explore)).toBe('codex/gpt-6.1-terra')
      expect(runFacts(view)).toBe('2 nodes · ≥17k tokens')
    })

    it('marks no bound on a running node, whose in-flight generation has no usage yet', () => {
      const view = buildRunView(
        { runId: 'r_e64a', at: 1 },
        parsed({
          status: 'running',
          nodes: { plan: 'done', 'explore-a': 'running' },
          node_details: {
            plan: {
              state: 'done',
              started_at: 1,
              usage: { input_tokens: 16464, output_tokens: 900, turns: 1 },
            },
            'explore-a': {
              state: 'running',
              started_at: 2,
              model: 'codex/gpt-6.1-terra',
              usage: {
                input_tokens: 30000,
                output_tokens: 6000,
                unreported_generations: 1,
                turns: 3,
              },
            },
          },
        }),
      )
      const [, explore] = view.phases.flatMap((phase) => phase.nodes)
      expect(explore.atLeast).toBe(false)
      expect(nodeFacts(explore)).toBe('codex/gpt-6.1-terra · 36k tokens')
      expect(runFacts(view)).not.toContain('≥')
    })
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
    expect(nodeMark({ ...pass, awaitingFunctions: true }, T + 7000 + 180_000)).toMatchObject({
      tone: 'accent',
      pulse: true,
    })
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
  const batch = () => buildRunView({ runId: 'r_3f03', at: 1 }, parsed(BATCH))

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

describe('phase rows', () => {
  const view = (value: unknown) => buildRunView({ runId: 'r', at: 1 }, parsed(value))
  const phaseOf = (run: ReturnType<typeof view>, name: string) => {
    const found = run.phases.find((phase) => phase.phase === name)
    if (!found) throw new Error(name)
    return found
  }
  /** Each phase's mark, whether it is current, whether it opens, and its facts. */
  const read = (run: ReturnType<typeof view>, name: string) => {
    const phase = phaseOf(run, name)
    return [phaseState(phase), phaseCurrent(run, phase), phaseOpens(run, phase), phaseFacts(phase)]
  }

  it('counts failed and running nodes per phase, from the status or the nodes', () => {
    const live = view(LIVE)
    expect(phaseOf(live, 'work')).toMatchObject({
      done: 1,
      total: 2,
      failed: 1,
      cancelled: 0,
      running: 0,
    })
    expect(phaseOf(live, 'check')).toMatchObject({ running: 1 })
    // `map` and the unphased group are not in the status's `phases`.
    expect(phaseOf(live, 'map')).toMatchObject({
      done: 0,
      total: 1,
      failed: 0,
      running: 0,
    })
    const loose = view({
      status: 'cancelled',
      nodes: { a: 'cancelled', b: 'running', c: 'done' },
      node_details: {
        a: { state: 'cancelled' },
        b: { state: 'running' },
        c: { state: 'done' },
      },
    })
    expect(phaseOf(loose, '')).toMatchObject({
      done: 1,
      total: 3,
      failed: 0,
      cancelled: 1,
      running: 1,
    })
  })

  it('marks each phase, and only a live run has a current one', () => {
    const live = view(LIVE)
    expect(read(live, 'work')).toEqual(['failed', false, true, '1 failed · 1/2'])
    expect(read(live, 'check')).toEqual(['running', true, true, '1/2'])
    expect(read(live, 'plan')).toEqual(['pending', false, false, '0/1'])
    expect(read(view(BATCH), 'test')).toEqual(['done', false, false, '1/1'])
    // Between two of its nodes, a phase stays current.
    const between = { ...phaseOf(live, 'plan'), done: 1, total: 3 }
    expect([phaseState(between), phaseCurrent(live, between)]).toEqual(['pending', true])
  })

  it('moves the current phase on, a failed one still open', () => {
    const kit = view(KIT)
    expect(read(kit, 'explore')).toEqual(['done', false, false, '2/2'])
    expect(read(kit, 'design')).toEqual(['running', true, true, '1 failed · 0/2'])
    expect(read(kit, 'refute')).toEqual(['pending', false, false, '0/2'])
    const next = view(KIT_NEXT)
    expect(read(next, 'design')).toEqual(['failed', false, true, '1 failed · 1/2'])
    expect(read(next, 'refute')).toEqual(['running', true, true, '0/2'])
  })

  it('keeps a phase current while its nodes wait on a retry', () => {
    // Both refute nodes timed out and went back to pending, sessions kept.
    const retry = view({
      ...KIT_NEXT,
      nodes: {
        ...KIT_NEXT.nodes,
        'refute-minimal': 'pending',
        'refute-robust': 'pending',
      },
      phases: {
        ...KIT_NEXT.phases,
        refute: { total: 2, done: 0, failed: 0, running: 0 },
      },
      node_details: {
        ...KIT_NEXT.node_details,
        'refute-minimal': {
          ...KIT_NEXT.node_details['refute-minimal'],
          state: 'pending',
        },
        // Reserved for its next attempt: a session, no start yet.
        'refute-robust': {
          ...KIT_NEXT.node_details['refute-robust'],
          session_id: 'wf_r_kit_refute-robust@r1',
        },
      },
    })
    expect(read(retry, 'refute')).toEqual(['pending', true, true, '0/2'])
    expect(read(retry, 'plan')).toEqual(['pending', false, false, '0/1'])
    // The phase the run waits at still leads with the going mark.
    expect(phaseMark(retry, phaseOf(retry, 'refute'))).toBe('running')
    expect(phaseMark(retry, phaseOf(retry, 'plan'))).toBe('pending')
  })

  it("leaves a loop's waiting phase pending while the other one runs", () => {
    // Pass 1 creates both members at once: verify#it1 waits on work#it1, no session.
    const pass = view({
      ...LIVE,
      nodes: { ...LIVE.nodes, 'work#it1': 'running', 'verify#it1': 'pending' },
      node_errors: {},
      phases: {
        ...LIVE.phases,
        work: { total: 2, done: 1, failed: 0, running: 1 },
        check: { total: 2, done: 1, failed: 0, running: 0 },
      },
      node_details: {
        ...LIVE.node_details,
        'work#it1': {
          state: 'running',
          phase: 'work',
          label: 'Work',
          started_at: T + 4000,
          session_id: 'wf_r_live_work#it1@r0',
        },
        'verify#it1': { state: 'pending', phase: 'check', label: 'Check' },
      },
    })
    expect(phaseMark(pass, phaseOf(pass, 'work'))).toBe('running')
    expect(phaseMark(pass, phaseOf(pass, 'check'))).toBe('pending')
  })

  it('leaves an ended run without a current phase', () => {
    // A tolerated failure in a finished run: open, not current.
    const done = view({ ...KIT_NEXT, status: 'completed' })
    expect(read(done, 'design')).toEqual(['failed', false, true, '1 failed · 1/2'])
    // A failed run left a phase part done: folded, not current.
    const failed = view({
      ...KIT,
      status: 'failed',
      phases: {
        ...KIT.phases,
        refute: { total: 3, done: 1, failed: 0, running: 0 },
      },
    })
    expect(read(failed, 'refute')).toEqual(['pending', false, false, '1/3'])
    expect(phaseMark(failed, phaseOf(failed, 'refute'))).toBe('pending')
    expect(phaseMark(done, phaseOf(done, 'design'))).toBe('failed')
  })

  it("tells a phase's failed nodes from its cancelled ones", () => {
    // design:robust failed, then design:minimal was cancelled.
    const design = (status: string) =>
      view({
        ...KIT,
        status,
        nodes: { ...KIT.nodes, 'design-minimal': 'cancelled' },
        phases: {
          ...KIT.phases,
          design: { total: 2, done: 0, failed: 2, cancelled: 1, running: 0 },
        },
        node_details: {
          ...KIT.node_details,
          'design-minimal': {
            ...KIT.node_details['design-minimal'],
            state: 'cancelled',
          },
        },
      })
    for (const status of ['failed', 'cancelled'])
      expect(read(design(status), 'design')).toEqual(['failed', false, true, '1 failed · 1 cancelled · 0/2'])
    // A failed run cancelled the nodes still going: their phase failed nothing.
    const failed = view({
      ...KIT_NEXT,
      status: 'failed',
      nodes: { ...KIT_NEXT.nodes, 'refute-minimal': 'cancelled' },
      phases: {
        ...KIT_NEXT.phases,
        refute: { total: 2, done: 0, failed: 1, cancelled: 1, running: 0 },
      },
      node_details: {
        ...KIT_NEXT.node_details,
        'refute-minimal': {
          ...KIT_NEXT.node_details['refute-minimal'],
          state: 'cancelled',
        },
      },
    })
    expect(read(failed, 'refute')).toEqual(['cancelled', false, false, '1 cancelled · 0/2'])
  })

  it('keeps a header for a phase whose nodes the status left out', () => {
    const { 'explore-code': _, 'explore-options': __, ...listed } = KIT_NEXT.node_details
    const cut = view({ ...KIT_NEXT, node_details: listed })
    expect(cut.phases.map((phase) => phase.phase)).toEqual(['explore', 'design', 'refute', 'plan'])
    expect(phaseOf(cut, 'explore').nodes).toEqual([])
    expect(read(cut, 'explore')).toEqual(['done', false, false, '2/2'])
    // The status tells a left-out node that failed from one the stop
    // cancelled, whatever the run's own end.
    const stopped = (cancelled: number) =>
      view({
        ...KIT_NEXT,
        status: 'cancelled',
        phases: {
          ...KIT_NEXT.phases,
          explore: { total: 2, done: 1, failed: 1, cancelled, running: 0 },
        },
        node_details: listed,
      })
    expect(read(stopped(1), 'explore')).toEqual(['cancelled', false, false, '1 cancelled · 1/2'])
    expect(read(stopped(0), 'explore')).toEqual(['failed', false, true, '1 failed · 1/2'])
    // A read without details shows no phases yet.
    expect(view({ ...KIT_NEXT, node_details: null }).phases).toEqual([])
  })

  it('groups a run with two phases or more', () => {
    expect(groupsByPhase(view(LIVE))).toBe(true)
    expect(groupsByPhase(view(KIT))).toBe(true)
    // One node per phase still groups, so a loop pass adds rows, not headers.
    expect(groupsByPhase(view(BATCH))).toBe(true)
    const run = (phases: Record<string, string | undefined>) =>
      view({
        status: 'running',
        nodes: Object.fromEntries(Object.keys(phases).map((uid) => [uid, 'pending'])),
        node_details: Object.fromEntries(
          Object.entries(phases).map(([uid, phase]) => [uid, { state: 'pending', phase }]),
        ),
      })
    expect(groupsByPhase(run({ a: 'x', b: 'x', c: 'y' }))).toBe(true)
    expect(groupsByPhase(run({ a: 'x', b: 'x', c: undefined }))).toBe(false)
    expect(groupsByPhase(view({ status: 'running', nodes: { a: 'pending' } }))).toBe(false)
  })

  it('reads a phase of one node as one line', () => {
    const kit = view(KIT)
    const plan = phaseOf(kit, 'plan')
    expect(solePhaseNode(plan)?.uid).toBe('plan')
    expect(solePhaseNode(phaseOf(kit, 'design'))).toBeNull()
    // A phase whose other nodes the status left out keeps its header.
    expect(solePhaseNode({ ...plan, total: 3 })).toBeNull()
    expect(solePhaseNode({ ...plan, nodes: [] })).toBeNull()
    // The node's own name stays when the phase does not already say it.
    const batch = view(BATCH)
    const [test] = phaseOf(batch, 'test').nodes
    const [check] = phaseOf(batch, 'check').nodes
    expect(phaseNodeLine(batch, test)).toBe('tests · codex/gpt-6-luna · 256k tokens')
    expect(phaseNodeLine(batch, check)).not.toMatch(/^check ·/)
    expect(phaseNodeLine(batch, check)).toMatch(/^codex\/gpt-6-luna · /)
    // A waiting node named like its phase has nothing to add.
    expect(phaseNodeLine(kit, plan.nodes[0])).toBe('')
  })

  it('leaves the phase out of a label under its header', () => {
    expect(underPhase('explore:code', 'explore')).toBe('code')
    expect(underPhase('Explore: code', 'explore')).toBe('code')
    expect(underPhase('verify:tests r1w1', 'verify')).toBe('tests r1w1')
    expect(underPhase('tests', 'test')).toBe('tests')
    expect(underPhase('plan', 'plan')).toBe('plan')
    expect(underPhase('KAN-12 · Title', 'work')).toBe('KAN-12 · Title')
    expect(underPhase('explore:code', '')).toBe('explore:code')
  })
})
