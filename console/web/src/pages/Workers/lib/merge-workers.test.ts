import { describe, expect, it } from 'vitest'
import type { RawWorkersSnapshot } from '../api/workers'
import { mergeWorkers } from './merge-workers'

function snapshot(partial: Partial<RawWorkersSnapshot>): RawWorkersSnapshot {
  return {
    engineWorkers: [],
    supervisorWorkers: [],
    configurations: [],
    infoByName: new Map(),
    ...partial,
  }
}

describe('mergeWorkers', () => {
  it('classifies config-managed workers', () => {
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
        configurations: [
          {
            id: 'harness',
            name: 'harness',
            description: '',
            schema: {},
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
    expect(rows[0]?.managementKind).toBe('config')
    expect(rows[0]?.stopEnabled).toBe(false)
    expect(rows[0]?.pid).toBe(100)
  })

  it('classifies supervisor-managed workers with stop enabled when running', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'w-2',
            name: 'iii-directory',
            runtime: 'rust',
            status: 'connected',
            function_count: 5,
            connected_at_ms: 0,
            active_invocations: 0,
            ip_address: '127.0.0.1',
          },
        ],
        supervisorWorkers: [
          {
            name: 'iii-directory',
            running: true,
            pid: 200,
            version: '0.1.0',
          },
        ],
        infoByName: new Map([
          [
            'iii-directory',
            {
              id: 'w-2',
              name: 'iii-directory',
              status: 'connected',
              function_count: 5,
              connected_at_ms: 0,
              active_invocations: 0,
              internal: false,
              pid: 200,
            },
          ],
        ]),
      }),
    )

    expect(rows[0]?.managementKind).toBe('supervisor')
    expect(rows[0]?.stopEnabled).toBe(true)
  })

  it('classifies standalone workers without stop', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [
          {
            id: 'w-3',
            name: 'todo-app',
            runtime: 'node',
            status: 'connected',
            function_count: 2,
            connected_at_ms: 0,
            active_invocations: 0,
          },
        ],
        infoByName: new Map([
          [
            'todo-app',
            {
              id: 'w-3',
              name: 'todo-app',
              status: 'connected',
              function_count: 2,
              connected_at_ms: 0,
              active_invocations: 0,
              internal: false,
              pid: 300,
            },
          ],
        ]),
      }),
    )

    expect(rows[0]?.managementKind).toBe('standalone')
    expect(rows[0]?.stopEnabled).toBe(false)
    expect(rows[0]?.stopDisabledReason).toContain('standalone')
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
    expect(rows[0]?.stopEnabled).toBe(false)
  })

  it('adds synthetic rows for supervisor-only daemon builtins', () => {
    const rows = mergeWorkers(
      snapshot({
        supervisorWorkers: [
          {
            name: 'iii-http',
            running: true,
            pid: 400,
            version: '0.1.0',
          },
        ],
      }),
    )

    expect(rows).toHaveLength(1)
    expect(rows[0]?.name).toBe('iii-http')
    expect(rows[0]?.managementKind).toBe('supervisor')
    expect(rows[0]?.runtime).toBeNull()
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
