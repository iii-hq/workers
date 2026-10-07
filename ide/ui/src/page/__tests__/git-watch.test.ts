import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  type GitChangedEvent,
  generation,
  publishWorktree,
  WORKTREE_COALESCE_MS,
  watchGit,
  watchWorktree,
} from '../git-watch'

function engine() {
  const handlers = new Map<string, (payload: unknown) => void>()
  const triggers: Array<{ type: string; function_id: string; config: Record<string, unknown> }> = []
  const host = {
    iii: {
      browserId: 'tab',
      on: (functionId: string, handler: (payload: unknown) => void) => {
        handlers.set(functionId, handler)
        return () => handlers.delete(functionId)
      },
      registerTrigger: (input: (typeof triggers)[number]) => {
        triggers.push(input)
        return () => triggers.splice(triggers.indexOf(input), 1)
      },
    },
  } as unknown as Host
  const bound = (type: string) => triggers.filter((t) => t.type === type)
  const fire = (type: string, payload: unknown) => {
    for (const t of bound(type)) handlers.get(t.function_id.replace(/::tab$/, ''))?.(payload)
  }
  return { host, handlers, bound, fire }
}

describe('watchGit', () => {
  it('binds one shell::git-changed per folder for every listener, until the last goes', () => {
    const { host, handlers, bound, fire } = engine()
    const heard: string[][] = []
    const a = watchGit(host, '/repo', (event) => heard.push(event.changes))
    const b = watchGit(host, '/repo', (event) => heard.push(event.changes))
    const other = watchGit(host, '/other', () => {})
    expect(bound('shell::git-changed').map((t) => t.config)).toEqual([{ path: '/repo' }, { path: '/other' }])
    const [repo] = bound('shell::git-changed')
    expect(repo.function_id).toMatch(/^iii::shell-ui::git-changed::\d+::tab$/)

    const before = generation(host, 'git', '/repo')
    fire('shell::git-changed', { path: '/repo', changes: ['head'] } satisfies GitChangedEvent)
    // Both listeners on the folder hear it; the other folder's does not.
    expect(heard).toEqual([['head'], ['head']])
    expect(generation(host, 'git', '/repo')).toBeGreaterThan(before)
    a()
    expect(bound('shell::git-changed')).toHaveLength(2)
    b()
    other()
    expect(bound('shell::git-changed')).toEqual([])
    expect(handlers.size).toBe(0)
  })

  it('ignores a payload that is not a batch of changes', () => {
    const { host, fire } = engine()
    const heard: unknown[] = []
    const off = watchGit(host, '/junk', (event) => heard.push(event))
    fire('shell::git-changed', { path: '/junk' })
    fire('shell::git-changed', null)
    expect(heard).toEqual([])
    off()
  })
})

describe('watchWorktree', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('folds a burst of file events into one notice, ignored paths left out', async () => {
    vi.useFakeTimers()
    const { host, bound, fire } = engine()
    let notices = 0
    const off = watchWorktree(host, '/tree', () => {
      notices += 1
    })
    expect(bound('shell::changed').map((t) => t.config)).toEqual([{ path: '/tree' }])
    fire('shell::changed', { path: 'target/x.o', kind: 'modified', root: '/tree', ignored: true })
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(notices).toBe(0)
    for (const path of ['a', 'b', 'c']) fire('shell::changed', { path, kind: 'modified', root: '/tree' })
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS - 1)
    expect(notices).toBe(0)
    await vi.advanceTimersByTimeAsync(1)
    expect(notices).toBe(1)
    off()
    expect(bound('shell::changed')).toEqual([])
  })

  it('makes no binding while a view passes the folder’s events on, and one once it stops', async () => {
    vi.useFakeTimers()
    const { host, bound } = engine()
    let notices = 0
    const page = publishWorktree(host, '/page')
    const off = watchWorktree(host, '/page', () => {
      notices += 1
    })
    expect(bound('shell::changed')).toEqual([])
    page.note()
    page.note()
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(notices).toBe(1)
    page.off()
    page.off()
    expect(bound('shell::changed').map((t) => t.config)).toEqual([{ path: '/page' }])
    // A view that comes back takes over again.
    const again = publishWorktree(host, '/page')
    expect(bound('shell::changed')).toEqual([])
    again.off()
    off()
    expect(bound('shell::changed')).toEqual([])
  })
})
