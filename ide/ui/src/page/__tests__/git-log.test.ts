import { describe, expect, it, vi } from 'vitest'
import {
  capText,
  EMPTY_TREE,
  gitDescribeChanges,
  gitLog,
  gitStashFiles,
  parseDecorations,
  parseLog,
  parseMessages,
  parseStashList,
  parseStashSubject,
  revisionParent,
  stashFileSource,
} from '../git-log'

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

function hostAnswering(answer: (args: string[]) => ReturnType<typeof reply>) {
  const calls: string[][] = []
  const trigger = vi.fn(async (_fn: string, payload: { args: string[] }) => {
    calls.push(payload.args)
    return answer(payload.args)
  })
  return { host: { iii: { trigger } } as unknown as Parameters<typeof gitLog>[0], calls }
}

const SHA_A = 'a'.repeat(40)
const SHA_B = 'b'.repeat(40)

describe('parseDecorations', () => {
  it('reads full ref names, keeping local and remote branches apart', () => {
    expect(
      parseDecorations(
        'HEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, tag: refs/tags/v1.2, refs/heads/feat/x, refs/stash',
      ),
    ).toEqual([
      { kind: 'head', name: 'main' },
      { kind: 'remote', name: 'origin/main' },
      { kind: 'tag', name: 'v1.2' },
      { kind: 'branch', name: 'feat/x' },
    ])
    expect(parseDecorations('HEAD')).toEqual([{ kind: 'head', name: 'HEAD' }])
    expect(parseDecorations('')).toEqual([])
  })
})

describe('parseLog', () => {
  it('splits records, parents and refs', () => {
    const stdout =
      `${SHA_A}\x00aaaaaaa\x00Ana\x00100\x00${SHA_B}\x00HEAD -> refs/heads/main\x00fix: one\x1e\n` +
      `${SHA_B}\x00bbbbbbb\x00Bo\x00 90\x00\x00\x00feat: root\x1e\n`
    const commits = parseLog(stdout)
    expect(commits).toHaveLength(2)
    expect(commits[0]).toMatchObject({
      sha: SHA_A,
      short: 'aaaaaaa',
      author: 'Ana',
      time: 100,
      parents: [SHA_B],
      subject: 'fix: one',
    })
    expect(commits[0].refs).toEqual([{ kind: 'head', name: 'main' }])
    expect(commits[1].parents).toEqual([])
    expect(revisionParent(commits[0])).toBe(SHA_B)
    expect(revisionParent(commits[1])).toBe(EMPTY_TREE)
  })

  it('rejects a malformed record', () => {
    expect(() => parseLog('nope\x00x\x1e')).toThrow(/malformed/)
  })
})

describe('gitLog', () => {
  it('asks for the first-parent history under the root with full decorations', async () => {
    const { host, calls } = hostAnswering(() => reply({ stdout: '' }))
    await gitLog(host, '/r', 50)
    expect(calls[0]).toEqual([
      'log',
      '--first-parent',
      '--max-count=50',
      '--decorate=full',
      '--format=%H%x00%h%x00%an%x00%ct%x00%P%x00%D%x00%s%x1e',
      'HEAD',
      '--',
      '.',
    ])
  })

  it('reads an unborn branch as no commits, and other failures as errors', async () => {
    const unborn = hostAnswering(() =>
      reply({ exit_code: 128, stderr: "fatal: your current branch 'main' does not have any commits yet" }),
    )
    await expect(gitLog(unborn.host, '/r')).resolves.toEqual([])
    const broken = hostAnswering(() => reply({ exit_code: 128, stderr: 'fatal: not a git repository' }))
    await expect(gitLog(broken.host, '/r')).rejects.toThrow('not a git repository')
  })
})

describe('stashes', () => {
  it('reads the branch and message out of the stash subject', () => {
    expect(parseStashSubject('On main: before the rebase')).toEqual({ branch: 'main', message: 'before the rebase' })
    expect(parseStashSubject('WIP on feat/x: 1a2b3c4d fix(ade): keep the pick')).toEqual({
      branch: 'feat/x',
      message: 'fix(ade): keep the pick',
    })
    expect(parseStashSubject('autostash')).toEqual({ branch: null, message: 'autostash' })
    expect(parseStashSubject('On (no branch): detached work')).toEqual({ branch: null, message: 'detached work' })
  })

  it('parses the list', () => {
    const stashes = parseStashList(
      `stash@{0}\x00${SHA_A}\x001700000000\x00On main: one\nstash@{1}\x00${SHA_B}\x001600000000\x00autostash\n`,
    )
    expect(stashes).toEqual([
      { ref: 'stash@{0}', sha: SHA_A, time: 1700000000, branch: 'main', message: 'one' },
      { ref: 'stash@{1}', sha: SHA_B, time: 1600000000, branch: null, message: 'autostash' },
    ])
  })
})

describe('the commit box helpers', () => {
  it('keeps distinct non-empty messages, newest first, up to the limit', () => {
    expect(parseMessages('fix: a\n\nbody\n\x1e\nfix: b\x1e\nfix: a\n\nbody\x1e\n\x1e', 5)).toEqual([
      'fix: a\n\nbody',
      'fix: b',
    ])
    expect(parseMessages('a\x1eb\x1ec', 2)).toEqual(['a', 'b'])
  })

  it('caps long text and says so', () => {
    expect(capText('short', 10)).toBe('short')
    expect(capText('0123456789abc', 10)).toBe('0123456789\n[diff truncated]')
  })
})

describe('gitDescribeChanges', () => {
  it('diffs tracked files against HEAD (renames by both names) and new files against /dev/null', async () => {
    const { host, calls } = hostAnswering((args) => {
      if (args[0] === 'rev-parse') return reply({ stdout: `${SHA_A}\n` })
      if (args.includes('--stat=100')) return reply({ stdout: ' a.ts | 2 +-\n' })
      if (args.includes('--no-index')) return reply({ exit_code: 1, stdout: '+++ b/new.md\n+hello\n' })
      return reply({ stdout: 'diff --git a/a.ts b/a.ts\n-x\n+y\n' })
    })
    const text = await gitDescribeChanges(host, '/r', [
      { path: 'a.ts', status: 'modified' },
      { path: 'b.ts', status: 'renamed', renameFrom: 'old.ts' },
      { path: 'new.md', status: 'untracked' },
    ])
    expect(calls.find((args) => args.includes('-U3'))).toEqual([
      'diff',
      '--no-ext-diff',
      '--find-renames',
      '-U3',
      'HEAD',
      '--',
      'a.ts',
      'old.ts',
      'b.ts',
    ])
    expect(calls.find((args) => args.includes('--no-index'))).toEqual([
      'diff',
      '--no-index',
      '--no-ext-diff',
      '--',
      '/dev/null',
      'new.md',
    ])
    expect(text).toContain('a.ts | 2 +-')
    expect(text).toContain('+y')
    expect(text).toContain('+hello')
  })

  it('compares against the empty tree before the first commit', async () => {
    const { host, calls } = hostAnswering((args) => (args[0] === 'rev-parse' ? reply({ exit_code: 1 }) : reply()))
    await gitDescribeChanges(host, '/r', [{ path: 'a.ts', status: 'added' }])
    expect(calls.find((args) => args.includes('-U3'))).toContain(EMPTY_TREE)
  })
})

describe('gitStashFiles', () => {
  it('lists tracked changes, then the unversioned files kept in the third parent', async () => {
    const { host } = hostAnswering((args) =>
      args[0] === 'ls-tree' ? reply({ stdout: 'new.md\x00notes/todo.txt\x00' }) : reply({ stdout: 'M\x00a.ts\x00' }),
    )
    expect(await gitStashFiles(host, '/r', SHA_A)).toEqual([
      { path: 'a.ts', status: 'modified' },
      { path: 'new.md', status: 'untracked', untracked: true },
      { path: 'notes/todo.txt', status: 'untracked', untracked: true },
    ])
  })

  it('reads a stash without a third parent as tracked changes only', async () => {
    const { host } = hostAnswering((args) =>
      args[0] === 'ls-tree'
        ? reply({ exit_code: 128, stderr: 'fatal: Not a valid object name' })
        : reply({ stdout: 'D\x00gone.ts\x00' }),
    )
    expect(await gitStashFiles(host, '/r', SHA_A)).toEqual([{ path: 'gone.ts', status: 'deleted' }])
  })

  it('opens a tracked file against the base and an unversioned one against nothing', () => {
    const stash = { sha: SHA_A, ref: 'stash@{0}' }
    expect(stashFileSource(stash, {})).toEqual({ type: 'revision', from: `${SHA_A}^1`, to: SHA_A, label: 'stash@{0}' })
    expect(stashFileSource(stash, { untracked: true })).toEqual({
      type: 'revision',
      from: EMPTY_TREE,
      to: `${SHA_A}^3`,
      label: 'stash@{0}',
    })
  })
})
