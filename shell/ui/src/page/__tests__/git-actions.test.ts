import { describe, expect, it, vi } from 'vitest'
import { discardStep, gitDiscard, gitFileAtRef, gitTags, gitUnstage, statusLetter } from '../git-actions'

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
    expect(discardStep({ path: 'n.ts', status: 'added', staged: true })).toEqual({ kind: 'unstage-delete', path: 'n.ts' })
    expect(discardStep({ path: 'b.ts', status: 'renamed', staged: true, from: 'a.ts' })).toEqual({
      kind: 'restore-rename',
      from: 'a.ts',
      path: 'b.ts',
    })
    expect(discardStep({ path: 'm.ts', status: 'modified', staged: false })).toEqual({ kind: 'restore', path: 'm.ts' })
  })
})

describe('gitDiscard', () => {
  it('restores tracked files from HEAD and reports per-file failures', async () => {
    const { host, calls } = hostWith(reply(), reply({ exit_code: 1, stderr: 'error: pathspec nope' }))
    const results = await gitDiscard(host, '/r', [
      { path: 'a.ts', status: 'modified', staged: false },
      { path: 'nope.ts', status: 'deleted', staged: false },
    ])
    expect(results).toEqual([
      { path: 'a.ts', error: null },
      { path: 'nope.ts', error: 'error: pathspec nope' },
    ])
    expect(calls[0]).toMatchObject({
      command: 'git',
      args: ['restore', '--source=HEAD', '--staged', '--worktree', '--', 'a.ts'],
      cwd: '/r',
    })
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
      'A', 'D', 'M', 'R', 'U',
    ])
  })
})
