import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  commitCommands,
  discardStep,
  gitCommitChanges,
  gitDiscard,
  gitFileAtRef,
  gitPush,
  gitStashPush,
  gitTags,
  gitUnstage,
  isIndexLocked,
  LOCK_RETRY_DELAYS_MS,
  stashPushArgs,
  statusLetter,
} from '../git-actions'

function reply(overrides: Partial<{ exit_code: number | null; stdout: string; stderr: string }> = {}) {
  return {
    exit_code: 0,
    stdout: '',
    stderr: '',
    timed_out: false,
    stdout_truncated: false,
    stderr_truncated: false,
    ...overrides,
  }
}

function hostWith(...responses: unknown[]) {
  const calls: unknown[] = []
  const trigger = vi.fn(async (_fn: string, payload: unknown) => {
    calls.push(payload)
    const next = responses.shift()
    if (next instanceof Error) throw next
    if (next === undefined) throw new Error('unexpected call')
    return next
  })
  return { host: { iii: { trigger } } as unknown as Parameters<typeof gitTags>[0], calls, trigger }
}

describe('discardStep', () => {
  it('deletes untracked, unstages-then-deletes added, restores the rest', () => {
    expect(discardStep({ path: 'n.ts', status: 'untracked', staged: false })).toEqual({ kind: 'delete', path: 'n.ts' })
    expect(discardStep({ path: 'n.ts', status: 'added', staged: true })).toEqual({
      kind: 'unstage-delete',
      path: 'n.ts',
    })
    expect(discardStep({ path: 'b.ts', status: 'renamed', staged: true, from: 'a.ts' })).toEqual({
      kind: 'restore-rename',
      from: 'a.ts',
      path: 'b.ts',
    })
    expect(discardStep({ path: 'm.ts', status: 'modified', staged: false })).toEqual({ kind: 'restore', path: 'm.ts' })
  })
})

describe('gitDiscard', () => {
  it('restores tracked files from HEAD and, when the batch fails, reports per-file failures', async () => {
    const { host, calls } = hostWith(
      reply({ exit_code: 1, stderr: 'error: pathspec nope' }),
      reply(),
      reply({ exit_code: 1, stderr: 'error: pathspec nope' }),
    )
    const results = await gitDiscard(host, '/r', [
      { path: 'a.ts', status: 'modified', staged: false },
      { path: 'nope.ts', status: 'deleted', staged: false },
    ])
    expect(results).toEqual([
      { path: 'a.ts', error: null },
      { path: 'nope.ts', error: 'error: pathspec nope' },
    ])
    expect(calls.map((call) => (call as { args: string[] }).args.slice(5))).toEqual([
      ['a.ts', 'nope.ts'],
      ['a.ts'],
      ['nope.ts'],
    ])
    expect(calls[1]).toMatchObject({
      command: 'git',
      args: ['restore', '--source=HEAD', '--staged', '--worktree', '--', 'a.ts'],
      cwd: '/r',
    })
  })

  it('discards many files in one call per step, in each change’s order', async () => {
    const { host, calls, trigger } = hostWith(reply(), reply(), {
      results: [{ success: true }, { success: false, error: { code: 'io', message: 'busy' } }, { success: true }],
    })
    const results = await gitDiscard(host, '/r', [
      { path: 'm.ts', status: 'modified', staged: false },
      { path: 'new.ts', status: 'untracked', staged: false },
      { path: 'add.ts', status: 'added', staged: true },
      { path: 'b.ts', status: 'renamed', staged: true, from: 'a.ts' },
    ])
    expect(trigger).toHaveBeenCalledTimes(3)
    expect((calls[0] as { args: string[] }).args.slice(4)).toEqual(['--', 'm.ts', 'a.ts'])
    expect((calls[1] as { args: string[] }).args).toEqual(['restore', '--staged', '--', 'add.ts', 'b.ts'])
    expect(calls[2]).toEqual({ paths: ['/r/new.ts', '/r/add.ts', '/r/b.ts'], recursive: false })
    expect(results).toEqual([
      { path: 'm.ts', error: null },
      { path: 'new.ts', error: null },
      { path: 'add.ts', error: 'busy' },
      { path: 'b.ts', error: null },
    ])
  })

  it('deletes untracked files through coder::delete-file', async () => {
    const { host, trigger } = hostWith({ results: [{ path: '/r/new.ts', success: true, removed: true }] })
    const results = await gitDiscard(host, '/r', [{ path: 'new.ts', status: 'untracked', staged: false }])
    expect(results[0].error).toBeNull()
    expect(trigger).toHaveBeenCalledWith('coder::delete-file', { paths: ['/r/new.ts'], recursive: false })
  })
})

describe('gitUnstage', () => {
  it('falls back to rm --cached when restore --staged fails (unborn HEAD)', async () => {
    const { host, calls } = hostWith(reply({ exit_code: 128, stderr: 'fatal: could not resolve HEAD' }), reply())
    await gitUnstage(host, '/r', ['a.ts'])
    expect((calls[1] as { args: string[] }).args).toEqual(['rm', '-q', '--cached', '-r', '--', 'a.ts'])
  })
})

describe('gitTags / gitFileAtRef', () => {
  it('parses tags newest first', async () => {
    const { host } = hostWith(reply({ stdout: 'v2\0bbbb\nv1\0aaaa\n' }))
    expect(await gitTags(host, '/r')).toEqual([
      { name: 'v2', sha: 'bbbb' },
      { name: 'v1', sha: 'aaaa' },
    ])
  })

  it('reads a file at a ref, treating a missing path as null and a bad ref as an error', async () => {
    const { host } = hostWith(
      reply({ stdout: 'body' }),
      reply({ exit_code: 128, stderr: "fatal: path 'x' exists on disk, but not in 'v1'" }),
      reply({ exit_code: 128, stderr: 'fatal: invalid object name zzz' }),
    )
    expect(await gitFileAtRef(host, '/r', 'v1', 'a.ts')).toBe('body')
    expect(await gitFileAtRef(host, '/r', 'v1', 'x')).toBeNull()
    await expect(gitFileAtRef(host, '/r', 'zzz', 'a.ts')).rejects.toThrow('unknown revision: zzz')
  })

  it('maps statuses to VS Code letters', () => {
    expect(['added', 'deleted', 'modified', 'renamed', 'untracked'].map((s) => statusLetter(s as never))).toEqual([
      'A',
      'D',
      'M',
      'R',
      'U',
    ])
  })
})

describe('commitCommands', () => {
  const base = {
    message: '  fix: a  ',
    paths: ['a.ts', 'new.md'],
    amend: false,
    signOff: false,
    noVerify: false,
    author: '',
  }

  it('adds the ticked paths, then commits exactly those paths', () => {
    expect(commitCommands(base)).toEqual([
      ['add', '-A', '--', 'a.ts', 'new.md'],
      ['commit', '-q', '-m', 'fix: a', '--', 'a.ts', 'new.md'],
    ])
  })

  it('carries amend, sign-off, --no-verify and the author', () => {
    const [, commit] = commitCommands({ ...base, amend: true, signOff: true, noVerify: true, author: ' Ana <a@x.io> ' })
    expect(commit).toEqual([
      'commit',
      '-q',
      '--amend',
      '--signoff',
      '--no-verify',
      '--author=Ana <a@x.io>',
      '-m',
      'fix: a',
      '--',
      'a.ts',
      'new.md',
    ])
  })

  it('amends only the message when nothing is ticked, and refuses an empty commit', () => {
    expect(commitCommands({ ...base, paths: [], amend: true })).toEqual([
      ['commit', '-q', '--amend', '--only', '-m', 'fix: a'],
    ])
    expect(() => commitCommands({ ...base, paths: [] })).toThrow('select the changes')
    expect(() => commitCommands({ ...base, message: '  ' })).toThrow('message is required')
  })
})

describe('gitDiscard keepAdded', () => {
  it('only un-adds an added file when told to keep local copies', async () => {
    const { host, calls } = hostWith(reply())
    const results = await gitDiscard(host, '/r', [{ path: 'n.ts', status: 'added', staged: true }], { keepAdded: true })
    expect(results).toEqual([{ path: 'n.ts', error: null }])
    expect(calls).toEqual([expect.objectContaining({ args: ['restore', '--staged', '--', 'n.ts'] })])
  })
})

describe('gitPush', () => {
  it('sets an upstream on origin when the branch has none', async () => {
    const { host, calls } = hostWith(
      reply({ exit_code: 128, stderr: 'fatal: The current branch feat has no upstream branch.' }),
      reply(),
    )
    await gitPush(host, '/r')
    expect(calls.map((call) => (call as { args: string[] }).args)).toEqual([
      ['push'],
      ['push', '--set-upstream', 'origin', 'HEAD'],
    ])
  })

  it('reports any other push failure', async () => {
    const { host } = hostWith(reply({ exit_code: 1, stderr: 'rejected: non-fast-forward' }))
    await expect(gitPush(host, '/r')).rejects.toThrow('non-fast-forward')
  })
})

describe('stashPushArgs', () => {
  it('stashes the whole tree, or only the given paths', () => {
    expect(stashPushArgs({ message: '', includeUntracked: false })).toEqual(['stash', 'push'])
    expect(stashPushArgs({ message: ' wip ', includeUntracked: true, paths: ['a.ts', 'new.md'] })).toEqual([
      'stash',
      'push',
      '--include-untracked',
      '-m',
      'wip',
      '--',
      'a.ts',
      'new.md',
    ])
  })
})

describe('index lock failures', () => {
  it("names a held index lock instead of git's bare error", async () => {
    expect(isIndexLocked('error: could not write index')).toBe(true)
    expect(isIndexLocked("fatal: Unable to create '/r/.git/index.lock': File exists.")).toBe(true)
    expect(isIndexLocked('rejected: non-fast-forward')).toBe(false)
    const { host } = hostWith(reply({ exit_code: 1, stderr: 'error: could not write index' }))
    await expect(gitPush(host, '/r')).rejects.toThrow('another git process kept this repository locked')
  })
})

describe('retrying a held index lock', () => {
  afterEach(() => vi.useRealTimers())
  const locked = () => reply({ exit_code: 128, stderr: "fatal: Unable to create '/r/.git/index.lock': File exists." })
  const commit = { message: 'fix: a', paths: ['a.ts'], amend: false, signOff: false, noVerify: false, author: '' }

  it('waits and runs the command again until the lock is free', async () => {
    vi.useFakeTimers()
    const { host, calls } = hostWith(locked(), locked(), reply(), reply(), reply({ stdout: 'abc1234\n' }))
    const done = gitCommitChanges(host, '/r', commit)
    await vi.runAllTimersAsync()
    await expect(done).resolves.toBe('abc1234')
    expect(calls.map((call) => (call as { args: string[] }).args[0])).toEqual([
      'add',
      'add',
      'add',
      'commit',
      'rev-parse',
    ])
  })

  it('gives up after the last wait, saying the repository stayed locked', async () => {
    vi.useFakeTimers()
    const { host, calls } = hostWith(...Array.from({ length: LOCK_RETRY_DELAYS_MS.length + 1 }, locked))
    const done = gitCommitChanges(host, '/r', commit)
    const outcome = expect(done).rejects.toThrow('kept this repository locked')
    await vi.runAllTimersAsync()
    await outcome
    expect(calls).toHaveLength(LOCK_RETRY_DELAYS_MS.length + 1)
  })

  it('stashes again only while no new stash entry exists', async () => {
    vi.useFakeTimers()
    const tip = (sha: string) => reply({ stdout: `${sha}\n` })
    // tip before, locked push, tip unchanged → retry, push succeeds.
    const ok = hostWith(tip('s1'), locked(), tip('s1'), reply({ stdout: 'Saved working directory' }))
    const first = gitStashPush(ok.host, '/r', { message: 'm', includeUntracked: false, paths: ['a.ts'] })
    await vi.runAllTimersAsync()
    await expect(first).resolves.toBeUndefined()
    expect(ok.calls).toHaveLength(4)
    // tip before, locked push, tip moved → the stash got stored: no second push.
    const moved = hostWith(tip('s1'), locked(), tip('s2'))
    const second = gitStashPush(moved.host, '/r', { message: 'm', includeUntracked: false, paths: ['a.ts'] })
    const outcome = expect(second).rejects.toThrow('kept this repository locked')
    await vi.runAllTimersAsync()
    await outcome
    expect(moved.calls).toHaveLength(3)
  })
})
