import { describe, expect, it } from 'vitest'
import type { SessionTriggerInfo } from '@/lib/backend/triggers'
import { buildTriggerWorkflow } from './SessionTriggers'

function trigger(over: Partial<SessionTriggerInfo>): SessionTriggerInfo {
  return {
    id: over.id ?? `t_${Math.random().toString(36).slice(2, 8)}`,
    triggerType: 'state',
    functionId: 'harness::react',
    config: {},
    configSummary: '',
    ...over,
  }
}

describe('buildTriggerWorkflow', () => {
  it('unconnected bindings have no structure and one level', () => {
    const wf = buildTriggerWorkflow([
      trigger({ id: 'a' }),
      trigger({ id: 'b', functionId: 'harness::notify_agent' }),
    ])
    expect(wf.hasStructure).toBe(false)
    expect(wf.levels).toHaveLength(1)
    expect(wf.levels[0]).toHaveLength(2)
  })

  it('groups join members under one unit', () => {
    const wf = buildTriggerWorkflow([
      trigger({
        id: 'm1',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'summarizer-x1' },
        metadata: {
          join: { id: 'analysts', expect: ['sum', 'fact'], key: 'sum' },
        },
      }),
      trigger({
        id: 'm2',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'factextractor-y2' },
        metadata: {
          join: { id: 'analysts', expect: ['sum', 'fact'], key: 'fact' },
        },
      }),
    ])
    expect(wf.hasStructure).toBe(true)
    expect(wf.levels).toHaveLength(1)
    const [unit] = wf.levels[0]
    expect(unit.join?.id).toBe('analysts')
    expect(unit.join?.expect).toEqual(['sum', 'fact'])
    expect(unit.members.map((m) => m.id)).toEqual(['m1', 'm2'])
  })

  it('levels chains: spawn target feeds completion watcher', () => {
    const wf = buildTriggerWorkflow([
      // stage 0: state change spawns the analyst into a named session
      trigger({ id: 'root', metadata: { session_id: 'analyst-1' } }),
      // stage 1: watches that session's turn completing, joins a barrier
      trigger({
        id: 'watcher',
        triggerType: 'harness::turn-completed',
        config: { session_id: 'analyst-1' },
        metadata: { join: { id: 'j', expect: ['a'], key: 'a' } },
      }),
    ])
    expect(wf.hasStructure).toBe(true)
    expect(wf.levels).toHaveLength(2)
    expect(wf.levels[0][0].members[0].id).toBe('root')
    expect(wf.levels[1][0].join?.id).toBe('j')
  })

  it('a watch cycle collapses instead of hanging', () => {
    const wf = buildTriggerWorkflow([
      trigger({
        id: 'a',
        triggerType: 'harness::turn-completed',
        config: { session_id: 's-b' },
        metadata: { session_id: 's-a' },
      }),
      trigger({
        id: 'b',
        triggerType: 'harness::turn-completed',
        config: { session_id: 's-a' },
        metadata: { session_id: 's-b' },
      }),
    ])
    expect(wf.levels.flat()).toHaveLength(2)
  })
})
