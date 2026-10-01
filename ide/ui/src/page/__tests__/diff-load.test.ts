import { describe, expect, it, vi } from 'vitest'

import { createTurnCache, loadDiffContents, loadTurnDiff, preImageBody, sameDiffContents, turnFileFor } from '../diff-load'
import type { SessionTurn } from '../turns'

function exec(overrides: Partial<{ exit_code: number; stdout: string; stderr: string }> = {}) {
  return { exit_code: 0, stdout: '', stderr: '', timed_out: false, stdout_truncated: false, stderr_truncated: false, ...overrides }
}

function hostWith(handlers: Record<string, (payload: Record<string, unknown>) => unknown>) {
  const trigger = vi.fn(async (fn: string, payload: Record<string, unknown>) => {
    const handler = handlers[fn]
    if (!handler) throw new Error(`unexpected ${fn}`)
    return handler(payload)
  })
  return { host: { iii: { trigger } } as unknown as Parameters<typeof loadDiffContents>[0], trigger }
}

const noTurns = { get: async () => null }

describe('preImageBody', () => {
  it('maps the stored pre-image onto a body or null', () => {
    expect(preImageBody({ content: 'x' })).toBe('x')
    expect(preImageBody({ missing: true })).toBe('')
    expect(preImageBody({ truncated: true, content: 'x' })).toBeNull()
    expect(preImageBody({ binary: true })).toBeNull()
    expect(preImageBody(null)).toBeNull()
  })
})

describe('loadDiffContents', () => {
  it('reads HEAD and the index for a staged diff, treating an unborn HEAD as empty', async () => {
    const { host } = hostWith({
      'shell::exec': ({ args }) => {
        const spec = (args as string[])[1]
        if (spec.startsWith('HEAD:')) return exec({ exit_code: 128, stderr: "fatal: invalid object name 'HEAD'" })
        return exec({ stdout: 'indexed\n' })
      },
    })
    expect(await loadDiffContents(host, '/r', 'a.ts', { type: 'staged' }, noTurns)).toEqual({
      oldContents: '',
      newContents: 'indexed\n',
    })
  })

  it('reads the index and the working copy for an unstaged diff, with an absent index as added', async () => {
    const { host } = hostWith({
      'shell::exec': () => exec({ exit_code: 128, stderr: "fatal: path 'a.ts' exists on disk, but not in the index" }),
      'coder::read-file': () => ({ content: 'new body', revision: 'r1' }),
    })
    expect(await loadDiffContents(host, '/r', 'a.ts', { type: 'unstaged' }, noTurns)).toEqual({
      oldContents: '',
      newContents: 'new body',
      worktreeRevision: 'r1',
    })
  })

  it('reads an untracked file as added: git calls an absent `:./path` "ambiguous"', async () => {
    const { host } = hostWith({
      'shell::exec': ({ args }) =>
        exec({
          exit_code: 128,
          stderr: `fatal: ambiguous argument '${(args as string[])[1]}': unknown revision or path not in the working tree.\nUse '--' to separate paths from revisions, like this:\n'git <command> [<revision>...] -- [<file>...]'`,
        }),
      'coder::read-file': () => ({ content: 'new body', revision: 'r1' }),
    })
    expect(await loadDiffContents(host, '/r', 'src/new.ts', { type: 'unstaged' }, noTurns)).toEqual({
      oldContents: '',
      newContents: 'new body',
      worktreeRevision: 'r1',
    })
  })

  it('reads a staged deletion as removed: HEAD has the body, the index answers "ambiguous"', async () => {
    const { host } = hostWith({
      'shell::exec': ({ args }) => {
        const spec = (args as string[])[1]
        if (spec.startsWith('HEAD:')) return exec({ stdout: 'was here\n' })
        return exec({ exit_code: 128, stderr: `fatal: ambiguous argument '${spec}': unknown revision or path not in the working tree.` })
      },
    })
    expect(await loadDiffContents(host, '/r', 'gone.ts', { type: 'staged' }, noTurns)).toEqual({
      oldContents: 'was here\n',
      newContents: '',
    })
  })

  it('treats a deleted working copy as an empty new side', async () => {
    const { host } = hostWith({
      'shell::exec': () => exec({ stdout: 'old' }),
      'coder::read-file': () => {
        throw new Error('handler error: {"code":"C211","message":"not found or not accessible"}')
      },
    })
    expect(await loadDiffContents(host, '/r', 'a.ts', { type: 'unstaged' }, noTurns)).toEqual({
      oldContents: 'old',
      newContents: '',
      worktreeRevision: undefined,
    })
  })

  it('flags a bad compare revision', async () => {
    const { host } = hostWith({
      'shell::exec': () => exec({ exit_code: 128, stderr: 'fatal: invalid object name zzz' }),
      'coder::read-file': () => ({ content: 'x' }),
    })
    await expect(loadDiffContents(host, '/r', 'a.ts', { type: 'compare', ref: 'zzz' }, noTurns)).rejects.toThrow(
      'unknown revision: zzz',
    )
  })

  it('reads a commit against its parent, a rename from its source, and nothing before a root commit', async () => {
    const specs: string[] = []
    const { host } = hostWith({
      'shell::exec': ({ args }) => {
        const spec = (args as string[])[1]
        specs.push(spec)
        if (spec === 'p1:old/a.ts') return exec({ stdout: 'before\n' })
        if (spec === 'c1:./a.ts') return exec({ stdout: 'after\n' })
        return exec({ exit_code: 128, stderr: `fatal: path 'a.ts' does not exist in '${spec.split(':')[0]}'` })
      },
    })
    const renamed = { type: 'commit', sha: 'c1', parent: 'p1', from: 'old/a.ts' } as const
    expect(await loadDiffContents(host, '/r', 'a.ts', renamed, noTurns)).toEqual({
      oldContents: 'before\n',
      newContents: 'after\n',
    })
    expect(await loadDiffContents(host, '/r', 'a.ts', { type: 'commit', sha: 'c1', parent: null }, noTurns)).toEqual({
      oldContents: '',
      newContents: 'after\n',
    })
    expect(specs).toEqual(['p1:old/a.ts', 'c1:./a.ts', 'c1:./a.ts'])
  })

  it('reads recorded changes through coder::change-diff', async () => {
    const { host } = hostWith({
      'coder::change-diff': () => ({ path: '/r/a.ts', old_contents: 'a', new_contents: 'b', is_binary: false }),
    })
    expect(await loadDiffContents(host, '/r', 'a.ts', { type: 'change', changeId: 'c1' }, noTurns)).toEqual({
      oldContents: 'a',
      newContents: 'b',
    })
  })
})

describe('loadTurnDiff', () => {
  const turn = (files: SessionTurn['files']): SessionTurn => ({ turn_id: 't', started_at: 1, files })
  const record = (overrides: Partial<SessionTurn['files'][number]>): SessionTurn['files'][number] => ({
    path: '/r/a.ts',
    kind: 'modified',
    cause: 'coder::update-file',
    first_seen: 1,
    last_seen: 1,
    ...overrides,
  })

  it('uses the stored before and the next turn pre-image as after', async () => {
    const { host, trigger } = hostWith({})
    const out = await loadTurnDiff(host, '/r', 'a.ts', turn([record({ before: { content: 'v1' }, after: { content: 'v2' } })]))
    expect(out).toEqual({ oldContents: 'v1', newContents: 'v2', note: undefined, worktreeRevision: undefined })
    expect(trigger).not.toHaveBeenCalled()
  })

  it('falls back to the working copy when no later turn kept the body', async () => {
    const { host } = hostWith({ 'coder::read-file': () => ({ content: 'now', revision: 'r9' }) })
    const out = await loadTurnDiff(host, '/r', 'a.ts', turn([record({ before: { content: 'v1' } })]))
    expect(out).toMatchObject({ oldContents: 'v1', newContents: 'now', worktreeRevision: 'r9' })
  })

  it('a watcher-observed creation has an empty before; a deletion an empty after', async () => {
    const { host } = hostWith({ 'coder::read-file': () => ({ content: 'made' }) })
    expect(await loadTurnDiff(host, '/r', 'a.ts', turn([record({ kind: 'created' })]))).toMatchObject({ oldContents: '', newContents: 'made' })
    expect(await loadTurnDiff(host, '/r', 'a.ts', turn([record({ kind: 'deleted', before: { content: 'gone' } })]))).toMatchObject({
      oldContents: 'gone',
      newContents: '',
    })
  })

  it('compares against the last commit when the pre-image was not kept, and says so', async () => {
    const { host } = hostWith({
      'shell::exec': () => exec({ stdout: 'committed' }),
      'coder::read-file': () => ({ content: 'now' }),
    })
    const out = await loadTurnDiff(host, '/r', 'src/a.ts', turn([record({ path: '/r/src/a.ts', before: { truncated: true } })]))
    expect(out.oldContents).toBe('committed')
    expect(out.note?.headline).toContain('last commit')
  })

  it('reports nothing to diff for an unknown file or an uncommitted lost pre-image', async () => {
    const { host } = hostWith({ 'shell::exec': () => exec({ exit_code: 128, stderr: 'fatal: not in HEAD' }) })
    expect(await loadTurnDiff(host, '/r', 'zzz.ts', turn([]))).toMatchObject({ noBaseline: true })
    expect(await loadTurnDiff(host, '/r', 'a.ts', turn([record({ before: { truncated: true } })]))).toMatchObject({ noBaseline: true })
    expect(turnFileFor(turn([record({})]), '/r', 'a.ts')?.path).toBe('/r/a.ts')
  })
})

describe('createTurnCache', () => {
  it('shares one read per turn but asks again after a miss or a failure', async () => {
    const turn: SessionTurn = { turn_id: 't1', started_at: 1, files: [] }
    const answers: Array<() => unknown> = [
      () => {
        throw new Error('worker restarting')
      },
      () => ({ turn: null }),
      () => ({ turn }),
    ]
    const { host, trigger } = hostWith({ 'shell::turns::get': () => answers.shift()?.() })
    const cache = createTurnCache(host, 's1')
    expect(await cache.get('t1')).toBeNull()
    expect(await cache.get('t1')).toBeNull()
    expect(await cache.get('t1')).toBe(turn)
    expect(await cache.get('t1')).toBe(turn)
    expect(trigger).toHaveBeenCalledTimes(3)
  })
})

describe('commit-panel diff sources', () => {
  it('uncommitted reads HEAD against the working copy', async () => {
    const specs: string[] = []
    const { host } = hostWith({
      'shell::exec': (payload) => {
        specs.push((payload.args as string[])[1])
        return exec({ stdout: 'old\n' })
      },
      'coder::read-file': () => ({ content: 'new\n', is_utf8: true, more_lines: false }),
    })
    const out = await loadDiffContents(host, '/r', 'a.ts', { type: 'uncommitted' }, noTurns)
    expect(specs).toEqual(['HEAD:./a.ts'])
    expect(out).toMatchObject({ oldContents: 'old\n', newContents: 'new\n' })
  })

  it('revision reads both sides from git, an absent side as empty', async () => {
    const { host } = hostWith({
      'shell::exec': (payload) => {
        const spec = (payload.args as string[])[1]
        return spec.startsWith('p:')
          ? exec({ exit_code: 128, stderr: "fatal: path 'a.ts' does not exist in 'p'" })
          : exec({ stdout: 'added\n' })
      },
    })
    const out = await loadDiffContents(host, '/r', 'a.ts', { type: 'revision', from: 'p', to: 'c', label: 'c' }, noTurns)
    expect(out).toMatchObject({ oldContents: '', newContents: 'added\n' })
  })
})

describe('sameDiffContents', () => {
  it('holds for equal sides and caveat, not for any difference', () => {
    const base = { oldContents: 'a\n', newContents: 'b\n', note: { headline: 'h', tone: 'warn' as const } }
    expect(sameDiffContents(base, { ...base, note: { ...base.note } })).toBe(true)
    expect(sameDiffContents(base, { ...base, newContents: 'c\n' })).toBe(false)
    expect(sameDiffContents(base, { ...base, note: { headline: 'h' } })).toBe(false)
    expect(sameDiffContents(base, { ...base, note: undefined })).toBe(false)
    expect(sameDiffContents(base, { ...base, binary: true })).toBe(false)
  })
})
