import { describe, expect, it, vi } from 'vitest'
import {
  gitBranchComparison,
  gitChanges,
  gitHeadBaseline,
  gitCommitComparison,
  gitComparison,
  gitReadSource,
  gitRecentCommits,
  gitRefs,
} from '../git'

interface ExecReply {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
  stdout_truncated: boolean
  stderr_truncated: boolean
}

function reply(overrides: Partial<ExecReply> = {}): ExecReply {
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

function mockedHost(...responses: Array<unknown | Error>) {
  const trigger = vi.fn(async () => {
    const next = responses.shift()
    if (next instanceof Error) throw next
    if (next === undefined) throw new Error('unexpected shell::exec call')
    return next
  })
  return {
    host: { iii: { trigger } } as unknown as Parameters<typeof gitComparison>[0],
    trigger,
  }
}

function repoHost(status: string, prefix = '', ...afterStatus: unknown[]) {
  return mockedHost(
    reply({ stdout: 'true\n' }),
    reply({ stdout: prefix }),
    reply({ stdout: status }),
    ...afterStatus,
  )
}

function uncommittedHost(
  status: string,
  diff: string,
  prefix = '',
  ...afterDiff: unknown[]
) {
  return repoHost(
    status,
    prefix,
    reply({ stdout: `${'a'.repeat(40)}\n` }),
    reply({ stdout: diff }),
    ...afterDiff,
  )
}

describe('gitComparison', () => {
  it('compares HEAD to the worktree, retaining both status columns and rename origin', async () => {
    const status =
      [
        'RM packages/app/src/new.ts',
        'packages/app/src/old.ts',
        '?? packages/app/fresh.ts',
      ].join('\0') + '\0'
    const aggregate =
      ['R090', 'packages/app/src/old.ts', 'packages/app/src/new.ts'].join('\0') + '\0'
    const { host, trigger } = uncommittedHost(status, aggregate, 'packages/app/\n')

    const state = await gitComparison(host, '/repo/packages/app', 'uncommitted')

    expect(state).toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [
        {
          path: 'src/new.ts',
          status: 'modified',
          staged: true,
          x: 'R',
          y: 'M',
          from: 'src/old.ts',
          renameFrom: 'src/old.ts',
          before: { kind: 'head', path: 'src/old.ts' },
          after: { kind: 'worktree', path: 'src/new.ts' },
        },
        {
          path: 'fresh.ts',
          status: 'untracked',
          staged: false,
          x: '?',
          y: '?',
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'fresh.ts' },
        },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(3, 'shell::exec', {
      command: 'git',
      args: [
        'status',
        '--porcelain=v1',
        '-z',
        '--untracked-files=all',
        '--renames',
        '--',
        '.',
      ],
      cwd: '/repo/packages/app',
      timeout_ms: 15_000,
    })
  })

  it('omits MM when the worktree restores the exact HEAD content', async () => {
    const { host } = uncommittedHost('MM restored.ts\0', '')

    await expect(gitComparison(host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [],
    })
  })

  it('omits a staged deletion recreated with the exact HEAD bytes', async () => {
    const hash = 'd'.repeat(40)
    const status = ['D  recreated.txt', '?? recreated.txt'].join('\0') + '\0'
    const { host } = uncommittedHost(
      status,
      'D\0recreated.txt\0',
      '',
      reply({ stdout: `${hash}\n` }),
      reply({ stdout: `${hash}\0recreated.txt\0` }),
    )

    await expect(gitComparison(host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [],
    })
  })

  it('compares an unborn repository against an empty tree', async () => {
    const status = ['A  staged.ts', '?? loose.ts', 'AD transient.ts'].join('\0') + '\0'
    const host = repoHost(status, '', reply({ exit_code: 1 }))

    await expect(gitComparison(host.host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [
        {
          path: 'staged.ts',
          status: 'added',
          staged: true,
          x: 'A',
          y: ' ',
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'staged.ts' },
        },
        {
          path: 'loose.ts',
          status: 'untracked',
          staged: false,
          x: '?',
          y: '?',
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'loose.ts' },
        },
      ],
    })
  })

  it('limits unstaged comparisons to Y-column changes and includes untracked files', async () => {
    const status =
      [
        'M  staged-only.ts',
        ' M modified.ts',
        ' R renamed.ts',
        'old-name.ts',
        '?? new.ts',
      ].join('\0') + '\0'
    const { host } = repoHost(status)

    const state = await gitComparison(host, '/repo', 'unstaged')

    expect(state).toEqual({
      kind: 'ready',
      scope: 'unstaged',
      changes: [
        {
          path: 'modified.ts',
          status: 'modified',
          staged: false,
          x: ' ',
          y: 'M',
          before: { kind: 'index', path: 'modified.ts' },
          after: { kind: 'worktree', path: 'modified.ts' },
        },
        {
          path: 'renamed.ts',
          status: 'renamed',
          staged: false,
          x: ' ',
          y: 'R',
          from: 'old-name.ts',
          renameFrom: 'old-name.ts',
          before: { kind: 'index', path: 'old-name.ts' },
          after: { kind: 'worktree', path: 'renamed.ts' },
        },
        {
          path: 'new.ts',
          status: 'untracked',
          staged: false,
          x: '?',
          y: '?',
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'new.ts' },
        },
      ],
    })
  })

  it('limits staged comparisons to X-column changes with HEAD/index sources', async () => {
    const status =
      [
        'R  renamed.ts',
        'old-name.ts',
        'A  added.ts',
        'D  deleted.ts',
        ' M unstaged-only.ts',
        '?? untracked.ts',
      ].join('\0') + '\0'
    const { host } = repoHost(status)

    const state = await gitComparison(host, '/repo', 'staged')

    expect(state).toEqual({
      kind: 'ready',
      scope: 'staged',
      changes: [
        {
          path: 'renamed.ts',
          status: 'renamed',
          staged: true,
          x: 'R',
          y: ' ',
          from: 'old-name.ts',
          renameFrom: 'old-name.ts',
          before: { kind: 'head', path: 'old-name.ts' },
          after: { kind: 'index', path: 'renamed.ts' },
        },
        {
          path: 'added.ts',
          status: 'added',
          staged: true,
          x: 'A',
          y: ' ',
          before: { kind: 'empty' },
          after: { kind: 'index', path: 'added.ts' },
        },
        {
          path: 'deleted.ts',
          status: 'deleted',
          staged: true,
          x: 'D',
          y: ' ',
          before: { kind: 'head', path: 'deleted.ts' },
          after: { kind: 'empty' },
        },
      ],
    })
  })

  it('treats AD as net-zero only across the aggregate uncommitted boundary', async () => {
    const status = 'AD transient.ts\0'

    const uncommitted = uncommittedHost(status, '')
    await expect(gitComparison(uncommitted.host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [],
    })

    const staged = repoHost(status)
    await expect(gitComparison(staged.host, '/repo', 'staged')).resolves.toEqual({
      kind: 'ready',
      scope: 'staged',
      changes: [
        {
          path: 'transient.ts',
          status: 'added',
          staged: true,
          x: 'A',
          y: 'D',
          before: { kind: 'empty' },
          after: { kind: 'index', path: 'transient.ts' },
        },
      ],
    })

    const unstaged = repoHost(status)
    await expect(gitComparison(unstaged.host, '/repo', 'unstaged')).resolves.toEqual({
      kind: 'ready',
      scope: 'unstaged',
      changes: [
        {
          path: 'transient.ts',
          status: 'deleted',
          staged: true,
          x: 'A',
          y: 'D',
          before: { kind: 'index', path: 'transient.ts' },
          after: { kind: 'empty' },
        },
      ],
    })
  })

  it('coalesces a staged deletion and untracked recreation by display path', async () => {
    const status = ['D  recreated.txt', '?? recreated.txt'].join('\0') + '\0'

    const workHash = 'b'.repeat(40)
    const headHash = 'c'.repeat(40)
    const uncommitted = uncommittedHost(
      status,
      'D\0recreated.txt\0',
      '',
      reply({ stdout: `${workHash}\n` }),
      reply({ stdout: `${headHash}\0recreated.txt\0` }),
    )
    await expect(gitComparison(uncommitted.host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'ready',
      scope: 'uncommitted',
      changes: [
        {
          path: 'recreated.txt',
          status: 'modified',
          staged: true,
          x: 'D',
          y: '?',
          before: { kind: 'head', path: 'recreated.txt' },
          after: { kind: 'worktree', path: 'recreated.txt' },
        },
      ],
    })

    const unstaged = repoHost(status)
    await expect(gitComparison(unstaged.host, '/repo', 'unstaged')).resolves.toEqual({
      kind: 'ready',
      scope: 'unstaged',
      changes: [
        {
          path: 'recreated.txt',
          status: 'untracked',
          staged: false,
          x: '?',
          y: '?',
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'recreated.txt' },
        },
      ],
    })

    const explorer = repoHost(status)
    await expect(gitChanges(explorer.host, '/repo')).resolves.toEqual({
      kind: 'ready',
      changes: [
        {
          path: 'recreated.txt',
          status: 'modified',
          staged: true,
        },
      ],
    })
  })

  it('returns explicit errors for truncated and malformed status output', async () => {
    const truncated = mockedHost(
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: ' M partial', stdout_truncated: true }),
    )
    await expect(gitComparison(truncated.host, '/repo', 'unstaged')).resolves.toEqual({
      kind: 'error',
      message: 'git status stdout was truncated',
    })

    const malformed = repoHost('R  new.ts\0')
    await expect(gitComparison(malformed.host, '/repo', 'staged')).resolves.toEqual({
      kind: 'error',
      message: 'git status returned an incomplete rename record',
    })
  })

  it('distinguishes a non-repository from an exec failure', async () => {
    const outside = mockedHost(reply({ exit_code: 128, stderr: 'not a git repository' }))
    await expect(gitComparison(outside.host, '/tmp', 'uncommitted')).resolves.toEqual({
      kind: 'not-a-repo',
    })

    const rejectedRepo = mockedHost(reply({ exit_code: 128, stderr: 'fatal: detected dubious ownership' }))
    await expect(gitComparison(rejectedRepo.host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'error',
      message: 'fatal: detected dubious ownership',
    })

    const failed = mockedHost(new Error('shell worker disconnected'))
    await expect(gitComparison(failed.host, '/repo', 'uncommitted')).resolves.toEqual({
      kind: 'error',
      message: 'git execution failed: shell worker disconnected',
    })
  })
})

describe('gitReadSource', () => {
  it('resolves empty, HEAD, index, revision, and worktree descriptors', async () => {
    const empty = mockedHost()
    await expect(gitReadSource(empty.host, '/repo', { kind: 'empty' })).resolves.toBe('')
    expect(empty.trigger).not.toHaveBeenCalled()

    const head = mockedHost(reply({ stdout: 'committed\n' }))
    await expect(
      gitReadSource(head.host, '/repo', { kind: 'head', path: 'src/app.ts' }),
    ).resolves.toBe('committed\n')
    expect(head.trigger).toHaveBeenCalledWith('shell::exec', {
      command: 'git',
      args: ['show', 'HEAD:./src/app.ts'],
      cwd: '/repo',
      timeout_ms: 15_000,
    })

    const index = mockedHost(reply({ stdout: 'staged\n' }))
    await expect(
      gitReadSource(index.host, '/repo', { kind: 'index', path: 'src/app.ts' }),
    ).resolves.toBe('staged\n')
    expect(index.trigger).toHaveBeenCalledWith('shell::exec', {
      command: 'git',
      args: ['show', ':./src/app.ts'],
      cwd: '/repo',
      timeout_ms: 15_000,
    })

    const revision = mockedHost(reply({ stdout: 'historical\n' }))
    await expect(
      gitReadSource(revision.host, '/repo', {
        kind: 'revision',
        revision: 'abc123',
        path: 'src/app.ts',
      }),
    ).resolves.toBe('historical\n')
    expect(revision.trigger).toHaveBeenCalledWith('shell::exec', {
      command: 'git',
      args: ['show', 'abc123:./src/app.ts'],
      cwd: '/repo',
      timeout_ms: 15_000,
    })

    const worktree = mockedHost({ content: 'working\n', is_utf8: true, more_lines: false })
    await expect(
      gitReadSource(worktree.host, '/repo', { kind: 'worktree', path: 'src/app.ts' }),
    ).resolves.toBe('working\n')
    expect(worktree.trigger).toHaveBeenCalledWith('coder::read-file', {
      path: '/repo/src/app.ts',
    })
  })

  it('rejects binary, partial, failed, and truncated reads', async () => {
    const binary = mockedHost({ content: null, is_utf8: false, more_lines: false })
    await expect(
      gitReadSource(binary.host, '/repo', { kind: 'worktree', path: 'image.png' }),
    ).rejects.toThrow('binary file: image.png')

    const partial = mockedHost({ content: 'prefix', is_utf8: true, more_lines: true })
    await expect(
      gitReadSource(partial.host, '/repo', { kind: 'worktree', path: 'large.txt' }),
    ).rejects.toThrow('worktree read was truncated: large.txt')

    const missing = mockedHost(reply({ exit_code: 128, stderr: 'fatal: path does not exist' }))
    await expect(
      gitReadSource(missing.host, '/repo', { kind: 'head', path: 'missing.ts' }),
    ).rejects.toThrow('fatal: path does not exist')

    const truncated = mockedHost(reply({ stdout: 'prefix', stdout_truncated: true }))
    await expect(
      gitReadSource(truncated.host, '/repo', { kind: 'index', path: 'large.ts' }),
    ).rejects.toThrow('git show index stdout was truncated')

    const gitBinary = mockedHost(reply({ stdout: 'header\0binary' }))
    await expect(
      gitReadSource(gitBinary.host, '/repo', {
        kind: 'revision',
        revision: 'abc123',
        path: 'image.png',
      }),
    ).rejects.toThrow('binary file: image.png')

    const invalidUtf8 = mockedHost(reply({ stdout: 'invalid\uFFFDbytes' }))
    await expect(
      gitReadSource(invalidUtf8.host, '/repo', { kind: 'head', path: 'archive.bin' }),
    ).rejects.toThrow('binary file: archive.bin')
  })
})

describe('gitCommitComparison', () => {
  it('compares a normal commit with its first parent and preserves renames', async () => {
    const sha = '1'.repeat(40)
    const parentSha = '2'.repeat(40)
    const diff =
      [
        'R100',
        'packages/app/src/old.ts',
        'packages/app/src/new.ts',
        'M',
        'packages/app/src/edited.ts',
        'D',
        'packages/app/src/deleted.ts',
      ].join('\0') + '\0'
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ stdout: 'packages/app/\n' }),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `${sha} ${parentSha}\n` }),
      reply({ stdout: diff }),
    )

    await expect(gitCommitComparison(host, '/repo/packages/app', 'selected')).resolves.toEqual({
      kind: 'ready',
      scope: 'commit',
      sha,
      parentSha,
      changes: [
        {
          path: 'src/new.ts',
          status: 'renamed',
          staged: false,
          from: 'src/old.ts',
          renameFrom: 'src/old.ts',
          before: { kind: 'revision', revision: parentSha, path: 'src/old.ts' },
          after: { kind: 'revision', revision: sha, path: 'src/new.ts' },
        },
        {
          path: 'src/edited.ts',
          status: 'modified',
          staged: false,
          before: { kind: 'revision', revision: parentSha, path: 'src/edited.ts' },
          after: { kind: 'revision', revision: sha, path: 'src/edited.ts' },
        },
        {
          path: 'src/deleted.ts',
          status: 'deleted',
          staged: false,
          before: { kind: 'revision', revision: parentSha, path: 'src/deleted.ts' },
          after: { kind: 'empty' },
        },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(5, 'shell::exec', {
      command: 'git',
      args: [
        'diff',
        '--no-ext-diff',
        '--name-status',
        '-z',
        '--find-renames',
        parentSha,
        sha,
        '--',
        '.',
      ],
      cwd: '/repo/packages/app',
      timeout_ms: 15_000,
    })
  })

  it('compares a root commit with an empty tree', async () => {
    const sha = '3'.repeat(40)
    const tree = 'e'.repeat(40)
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `tree ${tree}\n\nroot commit\n` }),
      reply({ stdout: 'A\0README.md\0' }),
    )

    await expect(gitCommitComparison(host, '/repo', sha)).resolves.toEqual({
      kind: 'ready',
      scope: 'commit',
      sha,
      parentSha: null,
      changes: [
        {
          path: 'README.md',
          status: 'added',
          staged: false,
          before: { kind: 'empty' },
          after: { kind: 'revision', revision: sha, path: 'README.md' },
        },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(6, 'shell::exec', {
      command: 'git',
      args: [
        'diff-tree',
        '--root',
        '--no-commit-id',
        '--name-status',
        '-z',
        '-r',
        '--find-renames',
        sha,
        '--',
        '.',
      ],
      cwd: '/repo',
      timeout_ms: 15_000,
    })
  })

  it('fails closed when a shallow boundary hides the selected commit parent', async () => {
    const sha = '4'.repeat(40)
    const parentSha = '5'.repeat(40)
    const tree = 'f'.repeat(40)
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `tree ${tree}\nparent ${parentSha}\n\nshallow boundary\n` }),
    )

    await expect(gitCommitComparison(host, '/repo', sha)).resolves.toEqual({
      kind: 'error',
      message: 'git selected commit parent is unavailable; fetch more history',
    })
    expect(trigger).toHaveBeenCalledTimes(5)
  })

  it('rejects malformed rename records', async () => {
    const sha = '6'.repeat(40)
    const parentSha = '7'.repeat(40)
    const { host } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: `${sha}\n` }),
      reply({ stdout: `${sha} ${parentSha}\n` }),
      reply({ stdout: 'R100\0old.ts\0' }),
    )
    await expect(gitCommitComparison(host, '/repo', sha)).resolves.toEqual({
      kind: 'error',
      message: 'git diff returned an incomplete rename record',
    })
  })
})

describe('gitBranchComparison', () => {
  it('compares merge base to the worktree, preserving renames and adding untracked files', async () => {
    const baseSha = '6'.repeat(40)
    const headSha = '7'.repeat(40)
    const mergeBase = '8'.repeat(40)
    const diff =
      [
        'R091',
        'src/old.ts',
        'src/new.ts',
        'M',
        'src/edited.ts',
        'A',
        'src/added.ts',
      ].join('\0') + '\0'
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: `${baseSha}\n` }),
      reply({ stdout: `${headSha}\n` }),
      reply({ stdout: `${mergeBase}\n` }),
      reply({ stdout: diff }),
      reply({ stdout: 'src/untracked.ts\0' }),
    )

    await expect(gitBranchComparison(host, '/repo', 'origin/main')).resolves.toEqual({
      kind: 'ready',
      scope: 'branch',
      baseRef: 'origin/main',
      baseSha,
      headSha,
      mergeBase,
      changes: [
        {
          path: 'src/new.ts',
          status: 'renamed',
          staged: false,
          from: 'src/old.ts',
          renameFrom: 'src/old.ts',
          before: { kind: 'revision', revision: mergeBase, path: 'src/old.ts' },
          after: { kind: 'worktree', path: 'src/new.ts' },
        },
        {
          path: 'src/edited.ts',
          status: 'modified',
          staged: false,
          before: { kind: 'revision', revision: mergeBase, path: 'src/edited.ts' },
          after: { kind: 'worktree', path: 'src/edited.ts' },
        },
        {
          path: 'src/added.ts',
          status: 'added',
          staged: false,
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'src/added.ts' },
        },
        {
          path: 'src/untracked.ts',
          status: 'untracked',
          staged: false,
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: 'src/untracked.ts' },
        },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(5, 'shell::exec', {
      command: 'git',
      args: ['merge-base', baseSha, headSha],
      cwd: '/repo',
      timeout_ms: 15_000,
    })
    expect(trigger).toHaveBeenNthCalledWith(7, 'shell::exec', {
      command: 'git',
      args: ['ls-files', '--others', '--exclude-standard', '-z', '--', '.'],
      cwd: '/repo',
      timeout_ms: 15_000,
    })
  })

  it('fails closed on truncated diffs and malformed untracked output', async () => {
    const baseSha = '9'.repeat(40)
    const headSha = 'a'.repeat(40)
    const mergeBase = 'b'.repeat(40)
    const beforeDiff = [
      reply({ stdout: 'true\n' }),
      reply(),
      reply({ stdout: `${baseSha}\n` }),
      reply({ stdout: `${headSha}\n` }),
      reply({ stdout: `${mergeBase}\n` }),
    ]
    const truncated = mockedHost(
      ...beforeDiff,
      reply({ stdout: 'M\0src/partial', stdout_truncated: true }),
    )
    await expect(gitBranchComparison(truncated.host, '/repo', 'main')).resolves.toEqual({
      kind: 'error',
      message: 'git diff branch stdout was truncated',
    })

    const malformed = mockedHost(
      ...beforeDiff,
      reply(),
      reply({ stdout: 'src/untracked.ts' }),
    )
    await expect(gitBranchComparison(malformed.host, '/repo', 'main')).resolves.toEqual({
      kind: 'error',
      message: 'git ls-files returned an incomplete path record',
    })
  })
})

describe('git metadata', () => {
  it('returns recent commit ids and subjects and bounds the requested count', async () => {
    const first = 'a'.repeat(40)
    const second = 'b'.repeat(40)
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ stdout: `${first}\n` }),
      reply({ stdout: `${first}\0First subject\n${second}\0Second subject\n` }),
    )

    await expect(gitRecentCommits(host, '/repo', 1_000)).resolves.toEqual({
      kind: 'ready',
      commits: [
        { sha: first, subject: 'First subject' },
        { sha: second, subject: 'Second subject' },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(3, 'shell::exec', {
      command: 'git',
      args: ['log', '--max-count=100', '--format=%H%x00%s'],
      cwd: '/repo',
      timeout_ms: 15_000,
    })
  })

  it('returns no commits for an unborn repository', async () => {
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ exit_code: 1 }),
    )
    await expect(gitRecentCommits(host, '/repo')).resolves.toEqual({
      kind: 'ready',
      commits: [],
    })
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('lists local and remote refs while omitting symbolic remote HEAD aliases', async () => {
    const localSha = 'c'.repeat(40)
    const remoteSha = 'd'.repeat(40)
    const output =
      [
        `refs/heads/main\0${localSha}\0*\0`,
        `refs/remotes/origin/HEAD\0${remoteSha}\0 \0refs/remotes/origin/main`,
        `refs/remotes/origin/main\0${remoteSha}\0 \0`,
      ].join('\n') + '\n'
    const { host, trigger } = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ stdout: output }),
    )

    await expect(gitRefs(host, '/repo')).resolves.toEqual({
      kind: 'ready',
      refs: [
        {
          kind: 'local',
          name: 'main',
          fullName: 'refs/heads/main',
          sha: localSha,
          current: true,
        },
        {
          kind: 'remote',
          name: 'origin/main',
          fullName: 'refs/remotes/origin/main',
          sha: remoteSha,
          current: false,
        },
      ],
    })
    expect(trigger).toHaveBeenNthCalledWith(2, 'shell::exec', {
      command: 'git',
      args: [
        'for-each-ref',
        '--sort=refname',
        '--format=%(refname)%00%(objectname)%00%(HEAD)%00%(symref)',
        'refs/heads/',
        'refs/remotes/',
      ],
      cwd: '/repo',
      timeout_ms: 15_000,
    })
  })

  it('fails closed on malformed metadata', async () => {
    const commits = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ stdout: `${'a'.repeat(40)}\n` }),
      reply({ stdout: 'not-a-hash\0subject\n' }),
    )
    await expect(gitRecentCommits(commits.host, '/repo')).resolves.toEqual({
      kind: 'error',
      message: 'git log returned an invalid commit id',
    })

    const refs = mockedHost(
      reply({ stdout: 'true\n' }),
      reply({ stdout: `refs/tags/v1\0${'a'.repeat(40)}\0 \0\n` }),
    )
    await expect(gitRefs(refs.host, '/repo')).resolves.toEqual({
      kind: 'error',
      message: 'git for-each-ref returned an unexpected ref',
    })
  })
})

describe('gitHeadBaseline', () => {
  it("reads the committed body from the file's own directory", async () => {
    const { host, trigger } = mockedHost(reply({ stdout: 'committed\n' }))

    await expect(
      gitHeadBaseline(host, '/root/nested/src', 'app.ts'),
    ).resolves.toBe('committed\n')
    expect(trigger).toHaveBeenCalledWith('shell::exec', {
      command: 'git',
      args: ['show', 'HEAD:./app.ts'],
      cwd: '/root/nested/src',
      timeout_ms: 15_000,
    })
  })

  it('reports absence instead of an empty body', async () => {
    const untracked = mockedHost(
      reply({ exit_code: 128, stderr: "fatal: path 'app.ts' does not exist" }),
    )
    await expect(
      gitHeadBaseline(untracked.host, '/root', 'app.ts'),
    ).resolves.toBeNull()

    const truncated = mockedHost(
      reply({ stdout: 'partial', stdout_truncated: true }),
    )
    await expect(
      gitHeadBaseline(truncated.host, '/root', 'app.ts'),
    ).resolves.toBeNull()

    const binary = mockedHost(reply({ stdout: 'PNG\0data' }))
    await expect(
      gitHeadBaseline(binary.host, '/root', 'logo.png'),
    ).resolves.toBeNull()

    const failed = mockedHost(new Error('exec unavailable'))
    await expect(
      gitHeadBaseline(failed.host, '/root', 'app.ts'),
    ).resolves.toBeNull()
  })
})
