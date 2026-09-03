import { describe, expect, it } from 'vitest'
import { filterWorkerRows, type WorkerRow } from './types'

const rows: WorkerRow[] = [
  {
    id: '1',
    name: 'harness',
    runtime: 'rust',
    ipAddress: '127.0.0.1',
    version: '1.0',
    pid: 1,
    tag: 'agent',
    managementKind: 'supervisor',
    status: 'connected',
    stopEnabled: true,
    stopDisabledReason: null,
    composeState: null,
    lastError: null,
  },
  {
    id: '2',
    name: 'todo-app',
    runtime: 'node',
    ipAddress: null,
    version: '2.0',
    pid: 2,
    tag: 'dev',
    managementKind: 'standalone',
    status: 'connected',
    stopEnabled: false,
    stopDisabledReason: 'standalone',
    composeState: null,
    lastError: null,
  },
]

describe('filterWorkerRows', () => {
  it('filters by tag', () => {
    const filtered = filterWorkerRows(rows, {
      search: '',
      tag: 'dev',
      runtime: null,
      management: null,
    })
    expect(filtered).toHaveLength(1)
    expect(filtered[0]?.name).toBe('todo-app')
  })

  it('filters by search across fields', () => {
    const filtered = filterWorkerRows(rows, {
      search: '127.0.0.1',
      tag: null,
      runtime: null,
      management: null,
    })
    expect(filtered).toHaveLength(1)
    expect(filtered[0]?.name).toBe('harness')
  })
})
