import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { CommitDetails, LogFilter, RefsSnapshot } from '../git-log-window'
import { detailsFor, useCommitDetails, useGitLog } from '../use-git-log'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

// What the mocked reads saw: the refs' listing a read finds, how many ran,
// and the author filter of each first page of the log.
const git = vi.hoisted(() => ({
  signature: 'a',
  refs: 0,
  fail: false,
  notRepo: false,
  hold: false,
  pages: [] as Array<string | undefined>,
  branchReads: 0,
}))
vi.mock('../git-log-window', async (original) => ({
  ...(await original<typeof import('../git-log-window')>()),
  readRefs: async () => {
    git.refs += 1
    if (git.fail) throw new Error('engine hiccup')
    return git.notRepo ? null : snapshot(git.signature)
  },
  readLogPage: async (_host: Host, _root: string, _tips: string[], filter: LogFilter) => {
    git.pages.push(filter.author)
    // A page that never comes back: still in flight when the test moves on.
    if (git.hold) return new Promise<never>(() => {})
    return { commits: [], done: true }
  },
  readCommitDetails: async (_host: Host, _root: string, _prefix: string, sha: string) => commit(sha),
  readContainingBranches: async () => {
    git.branchReads += 1
    return { names: ['main'], total: 1, partial: false }
  },
  // gpg wedged (a stale keyboxd lock) for one commit: its check never ends.
  readSignature: async (_host: Host, _root: string, sha: string) =>
    sha === 'wedged' ? new Promise<never>(() => {}) : 'G',
}))

const host = {} as Host
const snapshot = (signature: string): RefsSnapshot => ({
  refs: [],
  head: 'h',
  shallow: false,
  prefix: '',
  truncated: false,
  signature,
})

const commit = (sha: string): CommitDetails => ({
  sha,
  parents: [],
  author: 'a',
  authorEmail: 'a@x',
  authorDate: 0,
  committer: 'a',
  committerEmail: 'a@x',
  committerDate: 0,
  message: 'm',
  files: [],
  truncated: false,
})

describe('the commit details shown', () => {
  it('is nothing, and not loading, without a commit to show', () => {
    expect(detailsFor(null, undefined, { key: 'k', details: commit('k'), error: null })).toEqual({
      details: null,
      loading: false,
      error: null,
    })
  })

  it('loads until a read for this key lands, ignoring one for another', () => {
    expect(detailsFor('k', undefined, null)).toMatchObject({ details: null, loading: true })
    expect(detailsFor('k', undefined, { key: 'j', details: commit('j'), error: null })).toMatchObject({
      details: null,
      loading: true,
      error: null,
    })
    expect(detailsFor('k', undefined, { key: 'k', details: commit('k'), error: null })).toMatchObject({
      details: { sha: 'k' },
      loading: false,
    })
    expect(detailsFor('k', undefined, { key: 'k', details: null, error: 'boom' })).toEqual({
      details: null,
      loading: false,
      error: 'boom',
    })
  })

  it('shows a cached commit at once, even while an older read is abandoned', () => {
    expect(detailsFor('k', commit('k'), { key: 'j', details: null, error: 'late' })).toEqual({
      details: commit('k'),
      loading: false,
      error: null,
    })
  })
})

describe('the log', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('reads the refs on a new filter, then the log once from what they say', async () => {
    vi.stubGlobal('window', new EventTarget())
    vi.stubGlobal('document', new EventTarget())
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
    const log = mount((filter: LogFilter) => useGitLog(host, '/log', 0, true, filter, null), {})
    await settle()
    expect(git.refs).toBe(1)
    expect(git.pages).toEqual([undefined])

    // A commit in the IDE's terminal moves the refs, unseen; then a filter.
    git.signature = 'b'
    log.rerender({ author: 'a' })
    await settle()
    expect(git.refs).toBe(2)
    expect(git.pages).toEqual([undefined, 'a'])
    expect(log.result.snapshot?.signature).toBe('b')

    // Unmoved refs: the new filter's log all the same, and the same snapshot.
    const shown = log.result.snapshot
    log.rerender({ author: 'b' })
    await settle()
    expect(git.refs).toBe(3)
    expect(git.pages).toEqual([undefined, 'a', 'b'])
    expect(log.result.snapshot).toBe(shown)

    // The refs read fails: the new filter still reads its first page.
    git.fail = true
    log.rerender({ author: 'c' })
    await settle()
    expect(git.refs).toBe(4)
    expect(git.pages).toEqual([undefined, 'a', 'b', 'c'])
    // …and its read, which went well, clears the error the refs read left.
    expect(log.result.error).toBeNull()
    git.fail = false
    log.unmount()
  })

  it('stops loading when the folder stops being a repository mid-read', async () => {
    vi.stubGlobal('window', new EventTarget())
    vi.stubGlobal('document', new EventTarget())
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0))
    git.hold = true
    const log = mount(() => useGitLog(host, '/gone', 0, true, {}, null), undefined)
    await settle()
    expect(log.result.loading).toBe(true)
    // The next refs read finds no repository while the first page still reads.
    git.notRepo = true
    log.result.refresh()
    await settle()
    expect(log.result.notRepo).toBe(true)
    expect(log.result.loading).toBe(false)
    git.hold = false
    git.notRepo = false
    log.unmount()
  })
})

describe('useCommitDetails', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('keeps what a pane shows after another pane evicts it from the shared cache', async () => {
    vi.useFakeTimers()
    const shown = snapshot('s')
    const pane = (sha: string) => useCommitDetails(host, '/details', shown, sha)
    const first = mount(pane, 'x')
    await vi.advanceTimersByTimeAsync(300)
    expect(first.result).toMatchObject({ details: { sha: 'x' }, branches: { names: ['main'] } })

    // The second pane on the same commit is served from the cache, and
    // keeps it once the first has looked at 200 others.
    const second = mount(pane, 'x')
    expect(second.result).toMatchObject({ details: { sha: 'x' }, branches: { names: ['main'] } })
    for (let at = 0; at < 200; at++) {
      first.rerender(`c${at}`)
      await vi.advanceTimersByTimeAsync(300)
    }
    expect(first.result.details?.sha).toBe('c199')
    second.rerender('x')
    expect(second.result).toMatchObject({ details: { sha: 'x' }, loading: false, branches: { names: ['main'] } })
    first.unmount()
    second.unmount()
  })

  it('shows the details without waiting for the signature check, which comes after', async () => {
    vi.useFakeTimers()
    const pane = mount((sha: string) => useCommitDetails(host, '/signed', snapshot('s'), sha), 'wedged')
    await vi.advanceTimersByTimeAsync(300)
    expect(pane.result).toMatchObject({ details: { sha: 'wedged' }, loading: false, signature: null })
    pane.rerender('signed')
    await vi.advanceTimersByTimeAsync(300)
    expect(pane.result).toMatchObject({ details: { sha: 'signed' }, signature: 'G' })
    pane.unmount()
  })

  it('reads the branches again only when the refs moved, and nothing before the refs are read', async () => {
    vi.useFakeTimers()
    const pane = mount(
      ({ refs, sha }: { refs: RefsSnapshot | null; sha: string }) => useCommitDetails(host, '/branches', refs, sha),
      { refs: null, sha: 'b1' },
    )
    // A Git window hidden across a root switch has no refs: it reads nothing.
    await vi.advanceTimersByTimeAsync(300)
    const before = git.branchReads
    expect(pane.result).toMatchObject({ details: null, branches: null, signature: null })
    pane.rerender({ refs: snapshot('listing'), sha: 'b1' })
    await vi.advanceTimersByTimeAsync(300)
    expect(git.branchReads).toBe(before + 1)
    // Refresh reads an equal listing into a new snapshot: the same branches.
    pane.rerender({ refs: snapshot('listing'), sha: 'b1' })
    expect(pane.result.branches).toMatchObject({ names: ['main'] })
    await vi.advanceTimersByTimeAsync(300)
    expect(git.branchReads).toBe(before + 1)
    // The refs moved: read again.
    pane.rerender({ refs: snapshot('moved'), sha: 'b1' })
    await vi.advanceTimersByTimeAsync(300)
    expect(git.branchReads).toBe(before + 2)
    pane.unmount()
  })
})
