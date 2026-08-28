import { describe, expect, it } from 'vitest'
import {
  dependenciesSatisfied,
  deriveRunStatus,
  executorLabel,
  executorsFromCatalog,
  runtimeCapabilities,
  singleFlight,
  type TaskRecord,
  taskCounts,
  validateCreateRun,
} from '../src/model.js'

function task(status: TaskRecord['status']): TaskRecord {
  return {
    id: status,
    run_id: 'run',
    title: status,
    instruction: status,
    executor_id: 'worker',
    executor_function: 'worker::task',
    executor_kind: 'task',
    status,
    attempt: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}

describe('deriveRunStatus', () => {
  it('keeps attention and active work ahead of review', () => {
    expect(deriveRunStatus([task('review'), task('needs_you')])).toBe('needs_you')
    expect(deriveRunStatus([task('review'), task('running')])).toBe('active')
  })

  it('marks a fully accepted run done', () => {
    expect(deriveRunStatus([task('done'), task('done')])).toBe('done')
  })
})

describe('taskCounts', () => {
  it('returns every board column including empty columns', () => {
    expect(taskCounts([task('running'), task('running')])).toEqual({
      needs_you: 0,
      queued: 0,
      running: 2,
      review: 0,
      ready: 0,
      done: 0,
    })
  })
})

describe('validateCreateRun', () => {
  it('normalizes text and defaults auto dispatch', () => {
    expect(
      validateCreateRun({ title: '  Release  ', tasks: [{ instruction: '  inspect  ', executor: ' worker ' }] }),
    ).toEqual({
      title: 'Release',
      isolation: 'shared',
      auto_dispatch: true,
      tasks: [{ key: 'task-1', instruction: 'inspect', executor: 'worker', depends_on: [] }],
    })
  })

  it('rejects empty task lists', () => {
    expect(() => validateCreateRun({ tasks: [] })).toThrow('tasks must contain at least one task')
  })

  it('validates a per-task worktree dependency graph', () => {
    expect(
      validateCreateRun({
        repo_path: '/repo',
        isolation: 'worktree_per_task',
        tasks: [
          { key: 'research', instruction: 'inspect', executor: 'harness' },
          { key: 'build', instruction: 'implement', executor: 'pi', depends_on: ['research'] },
        ],
      }),
    ).toMatchObject({
      repo_path: '/repo',
      isolation: 'worktree_per_task',
      tasks: [
        { key: 'research', depends_on: [] },
        { key: 'build', depends_on: ['research'] },
      ],
    })
  })

  it('requires a repository for isolated work', () => {
    expect(() =>
      validateCreateRun({
        isolation: 'worktree_per_task',
        tasks: [{ instruction: 'inspect', executor: 'harness' }],
      }),
    ).toThrow('repo_path is required')
  })

  it('rejects dependency cycles before any work is dispatched', () => {
    expect(() =>
      validateCreateRun({
        tasks: [
          { key: 'a', instruction: 'first', executor: 'harness', depends_on: ['b'] },
          { key: 'b', instruction: 'second', executor: 'harness', depends_on: ['a'] },
        ],
      }),
    ).toThrow('dependency cycle')
  })
})

describe('dependenciesSatisfied', () => {
  it('releases a dependent after its prerequisite reaches review', () => {
    const first = { ...task('review'), id: 'first' }
    const second = { ...task('queued'), id: 'second', depends_on: ['first'] }
    expect(dependenciesSatisfied(second, [first, second])).toBe(true)
    expect(dependenciesSatisfied(second, [{ ...first, status: 'running' }, second])).toBe(false)
  })
})

describe('singleFlight', () => {
  it('coalesces concurrent work for the same task and releases the key afterward', async () => {
    const inFlight = new Map<string, Promise<string>>()
    let calls = 0
    const work = async () => {
      calls += 1
      await Promise.resolve()
      return `run-${calls}`
    }

    const first = singleFlight(inFlight, 'task-1', work)
    const second = singleFlight(inFlight, 'task-1', work)
    expect(second).toBe(first)
    await expect(Promise.all([first, second])).resolves.toEqual(['run-1', 'run-1'])
    await expect(singleFlight(inFlight, 'task-1', work)).resolves.toBe('run-2')
  })
})

it('turns worker ids into readable labels', () => {
  expect(executorLabel('code-runner::task', 'code-runner')).toBe('Code Runner')
})

describe('executor discovery', () => {
  it('discovers task-contract workers and preserves their namespace', () => {
    expect(
      executorsFromCatalog([
        {
          function_id: 'claude-code::task',
          worker_name: 'claude-code',
          namespace: 'agents',
          description: 'Writes completion records to agent_tasks state.',
        },
        { function_id: 'claude-code::stop', namespace: 'agents' },
        { function_id: 'claude-code::status', namespace: 'other' },
      ]),
    ).toEqual([
      expect.objectContaining({
        id: 'claude-code',
        namespace: 'agents',
        stop_function: 'claude-code::stop',
      }),
    ])
  })

  it('only advertises Harness controls registered in the same namespace', () => {
    expect(
      executorsFromCatalog([
        { function_id: 'harness::spawn', worker_name: 'harness', namespace: 'runtime' },
        { function_id: 'harness::stop', namespace: 'other' },
      ]),
    ).toEqual([
      expect.not.objectContaining({
        stop_function: 'harness::stop',
      }),
    ])
  })

  it('reports the live runtime capabilities used by the board', () => {
    const catalog = [
      { function_id: 'harness::spawn' },
      { function_id: 'worktree::create' },
      { function_id: 'worktree::status' },
      { function_id: 'worktree::land' },
      {
        function_id: 'pi::task',
        worker_name: 'pi',
        description: 'Writes completion records to agent_tasks state.',
      },
    ]
    expect(runtimeCapabilities(catalog, executorsFromCatalog(catalog))).toEqual({
      harness: true,
      worktree: true,
      external_executors: 1,
    })
  })
})
