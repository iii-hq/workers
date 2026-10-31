import { describe, expect, it } from 'vitest'
import type { RawWorkersSnapshot } from '../api/workers'
import { mergeWorkers } from './merge-workers'

function snapshot(partial: Partial<RawWorkersSnapshot>): RawWorkersSnapshot {
  return {
    engineWorkers: [],
    infoByName: new Map(),
    compose: null,
    ...partial,
  }
}

describe('mergeWorkers', () => {
  it('classifies connected workers outside compose as standalone', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'w-1',
            name: 'harness',
            status: 'connected',
            function_count: 1,
            connected_at_ms: 0,
            active_invocations: 0,
          },
        ],
        infoByName: new Map([
          [
            'harness',
            {
              id: 'w-1',
              name: 'harness',
              status: 'connected',
              function_count: 1,
              connected_at_ms: 0,
              active_invocations: 0,
              internal: false,
              pid: 100,
            },
          ],
        ]),
      }),
    )

    expect(rows).toHaveLength(1)
    expect(rows[0]?.managementKind).toBe('standalone')
    expect(rows[0]?.pid).toBe(100)
  })

  it('treats the engine own available workers as connected', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'configuration',
            name: 'configuration',
            status: 'available',
            function_count: 8,
            connected_at_ms: 0,
            active_invocations: 0,
          },
        ],
      }),
    )

    expect(rows[0]?.status).toBe('connected')
  })

  it('classifies internal engine workers', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'w-4',
            name: 'iii-engine-functions',
            runtime: 'rust',
            status: 'connected',
            function_count: 10,
            connected_at_ms: 0,
            active_invocations: 0,
          },
        ],
        infoByName: new Map([
          [
            'iii-engine-functions',
            {
              id: 'w-4',
              name: 'iii-engine-functions',
              status: 'connected',
              function_count: 10,
              connected_at_ms: 0,
              active_invocations: 0,
              internal: true,
              pid: 1,
            },
          ],
        ]),
      }),
    )

    expect(rows[0]?.managementKind).toBe('internal')
  })

  it('carries optional tag from engine info or list row', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'w-5',
            name: 'console',
            runtime: 'rust',
            status: 'connected',
            function_count: 1,
            connected_at_ms: 0,
            active_invocations: 0,
            tag: 'platform',
          },
        ],
        infoByName: new Map([
          [
            'console',
            {
              id: 'w-5',
              name: 'console',
              status: 'connected',
              function_count: 1,
              connected_at_ms: 0,
              active_invocations: 0,
              internal: false,
              tag: 'agent',
            },
          ],
        ]),
      }),
    )

    expect(rows[0]?.tag).toBe('agent')
  })
})
