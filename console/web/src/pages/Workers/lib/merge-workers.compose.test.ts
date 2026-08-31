import { describe, expect, it } from 'vitest'
import type { RawWorkersSnapshot } from '../api/workers'
import type { WorkerRow } from '../types'
import { composeActions, distinctManagement, filterWorkerRows } from '../types'
import {
  mergeWorkers,
  mergeWorkersView,
  summarizeCompose,
} from './merge-workers'

function row(rows: WorkerRow[], name: string): WorkerRow {
  const found = rows.find((r) => r.name === name)
  if (!found) throw new Error(`no row named ${name}`)
  return found
}

function snapshot(partial: Partial<RawWorkersSnapshot>): RawWorkersSnapshot {
  return {
    engineWorkers: [],
    supervisorWorkers: [],
    configurations: [],
    infoByName: new Map(),
    compose: null,
    ...partial,
  }
}

const engineWorker = (name: string, status = 'connected') => ({
  id: `id-${name}`,
  name,
  status,
  function_count: 1,
  connected_at_ms: 0,
  active_invocations: 0,
})

const compose: RawWorkersSnapshot['compose'] = {
  namespace: 'my-project',
  file: '/proj/worker-compose.yaml',
  state_dir: '/home/me/.iii/compose/my-project/proj-1',
  daemon_pid: 27045,
  containers: [
    { container: 'llm-router', state: 'ready', owned: true, pid: 27610 },
    {
      container: 'provider-openai',
      state: 'starting',
      owned: true,
      pid: 27638,
    },
    {
      container: 'provider-anthropic',
      state: 'failed',
      owned: false,
      pid: 27640,
      last_error: 'exited with status 1',
    },
    { container: 'web', state: 'stopped', owned: false, pid: 69884 },
  ],
}

describe('mergeWorkers with compose', () => {
  it('marks connected containers as compose-supervised and keeps the engine pid', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [engineWorker('llm-router')],
        infoByName: new Map([
          [
            'llm-router',
            { ...engineWorker('llm-router'), internal: false, pid: 27610 },
          ],
        ]),
        compose,
      }),
    )
    const found = row(rows, 'llm-router')
    expect(found).toMatchObject({
      managementKind: 'compose',
      status: 'connected',
      composeState: 'ready',
      pid: 27610,
      stopEnabled: false,
      stopDisabledReason: null,
    })
    expect(composeActions(found)).toEqual(['stop', 'restart'])
  })

  it('compose wins over supervisor and config classification', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [engineWorker('llm-router')],
        supervisorWorkers: [
          { name: 'llm-router', running: true, pid: 1, version: '1' },
        ],
        configurations: [
          { id: 'llm-router', name: 'llm-router', description: '', schema: {} },
        ],
        compose,
      }),
    )
    expect(rows.find((r) => r.name === 'llm-router')).toMatchObject({
      managementKind: 'compose',
      configurationId: 'llm-router',
    })
  })

  it('adds declared containers the engine does not know as synthetic rows', () => {
    const rows = mergeWorkers(snapshot({ compose }))
    expect(rows.map((r) => r.name)).toEqual([
      'llm-router',
      'provider-anthropic',
      'provider-openai',
      'web',
    ])
    expect(rows.find((r) => r.name === 'provider-anthropic')).toMatchObject({
      id: 'compose:provider-anthropic',
      status: 'failed',
      composeState: 'failed',
      lastError: 'exited with status 1',
      pid: null,
    })
    expect(rows.find((r) => r.name === 'provider-openai')).toMatchObject({
      status: 'starting',
      pid: 27638,
    })
    expect(rows.find((r) => r.name === 'web')).toMatchObject({
      status: 'stopped',
      pid: null,
    })
  })

  it('offers start for stopped and failed containers, stop while running', () => {
    const rows = mergeWorkers(snapshot({ compose }))
    expect(composeActions(row(rows, 'web'))).toEqual(['start', 'restart'])
    expect(composeActions(row(rows, 'provider-anthropic'))).toEqual([
      'start',
      'restart',
    ])
    expect(composeActions(row(rows, 'provider-openai'))).toEqual([
      'stop',
      'restart',
    ])
  })

  it('offers no compose actions to rows compose does not supervise', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [engineWorker('todo-app')],
        compose,
      }),
    )
    expect(composeActions(row(rows, 'todo-app'))).toEqual([])
  })

  it('summarizes the compose project for the page header', () => {
    expect(summarizeCompose(compose)).toEqual({
      namespace: 'my-project',
      file: '/proj/worker-compose.yaml',
      daemonPid: 27045,
      ready: 1,
      total: 4,
    })
    expect(summarizeCompose(null)).toBeNull()
    expect(mergeWorkersView(snapshot({ compose })).compose?.total).toBe(4)
  })

  it('filters by management kind and lists the kinds present in a fixed order', () => {
    const rows = mergeWorkers(
      snapshot({
        engineWorkers: [engineWorker('todo-app')],
        compose,
      }),
    )
    expect(distinctManagement(rows)).toEqual(['compose', 'standalone'])
    expect(
      filterWorkerRows(rows, {
        search: '',
        tag: null,
        runtime: null,
        management: 'compose',
      }).map((r) => r.name),
    ).toEqual(['llm-router', 'provider-anthropic', 'provider-openai', 'web'])
    expect(
      filterWorkerRows(rows, {
        search: 'failed',
        tag: null,
        runtime: null,
        management: null,
      }).map((r) => r.name),
    ).toEqual(['provider-anthropic'])
  })
})
