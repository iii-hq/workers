import type { Host, ObserveSource } from '@iii-dev/console-ui'
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
  const observers: Array<ObserveSource & { target: string }> = []
  const host = {
    iii: {
      browserId: 'tab',
      on: (functionId: string, handler: (payload: unknown) => void) => {
        handlers.set(functionId, handler)
        return () => handlers.delete(functionId)
      },
      observe: (source: ObserveSource, target: string) => {
        const observer = { ...source, target }
        observers.push(observer)
        return () => observers.splice(observers.indexOf(observer), 1)
      },
    },
  } as unknown as Host
  const observed = (functionId: string) => observers.filter((o) => o.function_id === functionId)
  const fire = (functionId: string, payload: unknown) => {
    for (const o of observed(functionId)) handlers.get(o.target.replace(/::tab$/, ''))?.(payload)
  }
  return { host, handlers, observed, fire }
}

describe('watchGit', () => {
  it('observes shell::on-git-changed once per folder for every listener, until the last goes', () => {
    const { host, handlers, observed, fire } = engine()
    const heard: string[][] = []
    const a = watchGit(host, '/repo', (event) => heard.push(event.changes))
    const b = watchGit(host, '/repo', (event) => heard.push(event.changes))
    const other = watchGit(host, '/other', () => {})
    expect(observed('shell::on-git-changed').map((o) => o.args)).toEqual([{ path: '/repo' }, { path: '/other' }])
    const [repo] = observed('shell::on-git-changed')
    expect(repo.target).toMatch(/^iii::shell-ui::git-changed::\d+::tab$/)

    const before = generation(host, 'git', '/repo')
    fire('shell::on-git-changed', { path: '/repo', changes: ['head'] } satisfies GitChangedEvent)
    // Both listeners on the folder hear it; the other folder's does not.
    expect(heard).toEqual([['head'], ['head']])
    expect(generation(host, 'git', '/repo')).toBeGreaterThan(before)
    a()
    expect(observed('shell::on-git-changed')).toHaveLength(2)
    b()
    other()
    expect(observed('shell::on-git-changed')).toEqual([])
    expect(handlers.size).toBe(0)
  })

  it('ignores a payload that is not a batch of changes', () => {
    const { host, fire } = engine()
    const heard: unknown[] = []
    const off = watchGit(host, '/junk', (event) => heard.push(event))
    fire('shell::on-git-changed', { path: '/junk' })
    fire('shell::on-git-changed', null)
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
    const { host, observed, fire } = engine()
    let notices = 0
    const off = watchWorktree(host, '/tree', () => {
      notices += 1
    })
    expect(observed('shell::on-changed').map((o) => o.args)).toEqual([{ path: '/tree' }])
    fire('shell::on-changed', { path: 'target/x.o', kind: 'modified', root: '/tree', ignored: true })
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(notices).toBe(0)
    for (const path of ['a', 'b', 'c']) fire('shell::on-changed', { path, kind: 'modified', root: '/tree' })
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS - 1)
    expect(notices).toBe(0)
    await vi.advanceTimersByTimeAsync(1)
    expect(notices).toBe(1)
    off()
    expect(observed('shell::on-changed')).toEqual([])
  })

  it('makes no observer while a view passes the folder’s events on, and one once it stops', async () => {
    vi.useFakeTimers()
    const { host, observed } = engine()
    let notices = 0
    const page = publishWorktree(host, '/page')
    const off = watchWorktree(host, '/page', () => {
      notices += 1
    })
    expect(observed('shell::on-changed')).toEqual([])
    page.note()
    page.note()
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(notices).toBe(1)
    page.off()
    page.off()
    expect(observed('shell::on-changed').map((o) => o.args)).toEqual([{ path: '/page' }])
    // A view that comes back takes over again.
    const again = publishWorktree(host, '/page')
    expect(observed('shell::on-changed')).toEqual([])
    again.off()
    off()
    expect(observed('shell::on-changed')).toEqual([])
  })
})
