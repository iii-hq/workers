/* The Log's reads and the new worktree verbs against a real git, in
   throwaway repositories. */

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { loadDiffContents } from '../src/page/diff-load'
import { gitRevertChanges } from '../src/page/git-actions'
import {
  findCommit,
  type LogCommit,
  type LogFilter,
  logTips,
  readCommitDetails,
  readContainingBranches,
  readLogPage,
  readRefs,
} from '../src/page/git-log-window'
import {
  createBranch,
  createWorktree,
  fetchAll,
  listWorktrees,
  pruneWorktrees,
  pushBranch,
  pushTarget,
  renameBranch,
  updateBranch,
} from '../src/page/worktrees'
import { commit, env, host, sh } from './git-host'

let dir: string
let repo: string

beforeEach(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'ide-git-log-')))
  env.HOME = dir
  env.XDG_CONFIG_HOME = dir
  repo = join(dir, 'repo')
  sh(dir, 'init', '-q', '-b', 'main', repo)
})

afterEach(() => rmSync(dir, { recursive: true, force: true }))

/** Every commit `filter` leaves, read `size` at a time. */
async function readAll(h: Host, filter: LogFilter = {}, size = 1000): Promise<LogCommit[]> {
  const snapshot = await readRefs(h, repo)
  if (snapshot === null) throw new Error('not a repository')
  const tips = logTips(snapshot, null)
  const all: LogCommit[] = []
  for (let skip = 0; ; ) {
    const page = await readLogPage(h, repo, tips, filter, skip, size)
    all.push(...page.commits)
    skip += page.commits.length
    if (page.done) return all
  }
}

/** The host with every stdout cut to `cap` bytes, as the worker's output cap does. */
function capped(cap: number): Host {
  const inner = host as unknown as { iii: { trigger: (fn: string, payload: unknown) => Promise<unknown> } }
  return {
    iii: {
      trigger: async (fn: string, payload: unknown) => {
        const out = (await inner.iii.trigger(fn, payload)) as { stdout?: string; stdout_truncated?: boolean }
        if (typeof out.stdout !== 'string' || out.stdout.length <= cap) return out
        return { ...out, stdout: out.stdout.slice(0, cap), stdout_truncated: true }
      },
    },
  } as unknown as Host
}

describe('the log reads', () => {
  it('reads an empty repository as no commits, with no git log at all', async () => {
    const snapshot = await readRefs(host, repo)
    expect(snapshot).toMatchObject({ head: null, refs: [], shallow: false })
    expect(await readLogPage(host, repo, [], {}, 0)).toEqual({ commits: [], done: true })
    expect(await readRefs(host, dir)).toBeNull()
  })

  it('reads refs: current branch, upstream counts, worktree, annotated tag', async () => {
    const origin = join(dir, 'origin.git')
    commit(repo, 'a.txt', 'a\n', 'one')
    sh(dir, 'clone', '-q', '--bare', repo, origin)
    sh(repo, 'remote', 'add', 'origin', origin)
    sh(repo, 'fetch', '-q', 'origin')
    sh(repo, 'branch', '-q', '--set-upstream-to=origin/main', 'main')
    commit(repo, 'b.txt', 'b\n', 'two')
    sh(repo, 'tag', '-a', 'v1', '-m', 'release', 'HEAD~1')
    sh(repo, 'worktree', 'add', '-q', '-b', 'feat/x', join(dir, 'repo.feat-x'))
    const snapshot = await readRefs(host, repo)
    const byName = new Map(snapshot?.refs.map((ref) => [ref.name, ref]))
    expect(byName.get('main')).toMatchObject({ kind: 'local', current: true, upstream: 'origin/main', ahead: 1 })
    expect(byName.get('feat/x')).toMatchObject({ worktree: join(dir, 'repo.feat-x'), subject: 'two' })
    expect(byName.get('origin/main')?.kind).toBe('remote')
    expect(byName.get('v1')).toMatchObject({ kind: 'tag', sha: sh(repo, 'rev-parse', 'HEAD~1') })
    // origin/HEAD, a symbolic ref, is left out.
    expect([...byName.keys()].sort()).toEqual(['feat/x', 'main', 'origin/main', 'v1'])
  })

  it('pages exactly: small pages and cut outputs read the same commits as one read', async () => {
    for (let i = 0; i < 23; i += 1)
      commit(repo, 'f.txt', `${i}\n`, `commit ${i} with a longer subject to fill the page`)
    sh(repo, 'switch', '-q', '-c', 'side', 'HEAD~10')
    commit(repo, 's.txt', 's\n', 'side work')
    sh(repo, 'switch', '-q', 'main')
    sh(repo, 'merge', '-q', '--no-ff', '-m', 'merge side', 'side')
    const whole = await readAll(host)
    expect(whole).toHaveLength(25)
    expect((await readAll(host, {}, 10)).map((c) => c.sha)).toEqual(whole.map((c) => c.sha))
    const cut = await readAll(capped(900), {}, 1000)
    expect(cut.map((c) => c.sha)).toEqual(whole.map((c) => c.sha))
    // The merge's parents are in order, first parent first.
    const merge = whole[0]
    expect(merge.subject).toBe('merge side')
    expect(merge.parents).toEqual([sh(repo, 'rev-parse', 'HEAD^1'), sh(repo, 'rev-parse', 'HEAD^2')])
  })

  it('filters by message (literal), author, dates and paths, and finds a hash prefix', async () => {
    commit(repo, 'a.txt', 'a\n', 'fix: a.b literal')
    env.GIT_AUTHOR_NAME = 'Ana'
    // The date filter goes by commit date, as git log's does.
    env.GIT_AUTHOR_DATE = '2020-01-01T00:00:00Z'
    env.GIT_COMMITTER_DATE = '2020-01-01T00:00:00Z'
    commit(repo, 'b.txt', 'b\n', 'feat: axb')
    delete env.GIT_AUTHOR_DATE
    delete env.GIT_COMMITTER_DATE
    env.GIT_AUTHOR_NAME = 't'
    mkdirSync(join(repo, 'docs'))
    commit(repo, 'docs/c.md', 'c\n', 'docs: c')
    const subjects = async (filter: LogFilter) => (await readAll(host, filter)).map((c) => c.subject)
    expect(await subjects({ text: 'a.b' })).toEqual(['fix: a.b literal'])
    expect(await subjects({ text: 'a.b', regex: true })).toEqual(['feat: axb', 'fix: a.b literal'])
    expect(await subjects({ text: 'FIX', caseSensitive: true })).toEqual([])
    expect(await subjects({ author: 'Ana' })).toEqual(['feat: axb'])
    const cut = Date.parse('2021-01-01T00:00:00Z') / 1000
    expect(await subjects({ until: cut })).toEqual(['feat: axb'])
    // As in `git log --since`, the walk stops at the first older commit: the
    // back-dated one hides what lies behind it.
    expect(await subjects({ since: cut })).toEqual(['docs: c'])
    expect(await subjects({ paths: ['docs'] })).toEqual(['docs: c'])
    const head = sh(repo, 'rev-parse', 'HEAD')
    expect((await findCommit(host, repo, head.slice(0, 8)))?.sha).toBe(head)
    expect(await findCommit(host, repo, 'deadbeef')).toBeNull()
  })

  it('reads commit details: root, rename and merge files, message, branches that have it', async () => {
    mkdirSync(join(repo, 'sub'))
    commit(repo, 'sub/a.txt', 'a\n', 'root commit')
    const root = sh(repo, 'rev-parse', 'HEAD')
    sh(repo, 'mv', 'sub/a.txt', 'sub/b.txt')
    sh(repo, 'commit', '-q', '-m', 'rename it\n\nwith a body')
    sh(repo, 'switch', '-q', '-c', 'side')
    commit(repo, 'side.txt', 's\n', 'side')
    sh(repo, 'switch', '-q', 'main')
    commit(repo, 'm.txt', 'm\n', 'main moves')
    sh(repo, 'merge', '-q', '--no-ff', '-m', 'merge side', 'side')

    const first = await readCommitDetails(host, join(repo, 'sub'), 'sub/', root)
    expect(first.parents).toEqual([])
    expect(first.files).toEqual([{ path: 'sub/a.txt', status: 'added', rel: 'a.txt', view: 'a.txt' }])
    const renamed = await readCommitDetails(host, repo, '', sh(repo, 'rev-parse', 'HEAD~2'))
    expect(renamed.message).toBe('rename it\n\nwith a body')
    expect(renamed.files).toEqual([
      { path: 'sub/b.txt', status: 'renamed', from: 'sub/a.txt', rel: 'sub/b.txt', view: 'sub/b.txt' },
    ])
    const merge = await readCommitDetails(host, repo, '', sh(repo, 'rev-parse', 'HEAD'))
    // Against its first parent, the merge brings in the side branch's file.
    expect(merge.files.map((file) => file.path)).toEqual(['side.txt'])
    expect(merge.signature).toBe('N')
    expect(await readContainingBranches(host, repo, root)).toEqual({
      names: ['main', 'side'],
      total: 2,
      partial: false,
    })

    // A file of the rename commit, as its diff tab reads it.
    const diff = await loadDiffContents(
      host,
      repo,
      'sub/b.txt',
      { type: 'commit', sha: renamed.sha, parent: renamed.parents[0], from: 'sub/a.txt' },
      { get: async () => null },
    )
    expect(diff).toEqual({ oldContents: 'a\n', newContents: 'a\n' })

    // Seen from sub/, the merge's file lies outside: its diff still opens,
    // by a path that climbs out.
    const fromSub = await readCommitDetails(host, join(repo, 'sub'), 'sub/', merge.sha)
    expect(fromSub.files).toEqual([{ path: 'side.txt', status: 'added', rel: null, view: '../side.txt' }])
    const outside = await loadDiffContents(
      host,
      join(repo, 'sub'),
      '../side.txt',
      { type: 'commit', sha: merge.sha, parent: merge.parents[0] },
      { get: async () => null },
    )
    expect(outside.newContents).toBe('s\n')
    expect(outside.oldContents).toBe('')
  })

  it('knows a shallow clone and stops at its cut', async () => {
    for (let i = 0; i < 4; i += 1) commit(repo, 'f.txt', `${i}\n`, `c${i}`)
    const shallow = join(dir, 'shallow')
    sh(dir, 'clone', '-q', '--depth', '2', `file://${repo}`, shallow)
    const snapshot = await readRefs(host, shallow)
    expect(snapshot?.shallow).toBe(true)
    const page = await readLogPage(host, shallow, logTips(snapshot!, null), {}, 0)
    expect(page.commits.map((c) => c.subject)).toEqual(['c3', 'c2'])
    expect(page.commits[1].parents).toEqual([])
  })
})

describe('the new worktree verbs', () => {
  it('creates a worktree from a start point, and tracks a remote branch it starts from', async () => {
    commit(repo, 'a.txt', 'a\n', 'one')
    const one = sh(repo, 'rev-parse', 'HEAD')
    commit(repo, 'b.txt', 'b\n', 'two')
    const origin = join(dir, 'origin.git')
    sh(dir, 'clone', '-q', '--bare', repo, origin)
    sh(repo, 'remote', 'add', 'origin', origin)
    sh(origin, 'branch', 'remote-only', one)
    await fetchAll(host, await listWorktrees(host, repo))
    expect(sh(repo, 'rev-parse', 'origin/remote-only')).toBe(one)

    let list = await listWorktrees(host, repo)
    const fromOne = await createWorktree(host, list, 'from-one', one)
    expect(sh(fromOne.path, 'rev-parse', 'HEAD')).toBe(one)
    list = await listWorktrees(host, repo)
    const tracked = await createWorktree(host, list, 'remote-only', 'origin/remote-only')
    expect(sh(tracked.path, 'rev-parse', '--abbrev-ref', '@{upstream}')).toBe('origin/remote-only')
    list = await listWorktrees(host, repo)
    expect(list.worktrees.find((wt) => wt.path === tracked.path)?.head).toBe(one)
    // Each row carries its last commit.
    expect(list.worktrees.find((wt) => wt.path === tracked.path)?.tip?.subject).toBe('one')
    expect(list.worktrees[0].tip?.subject).toBe('two')
    // A start point is for a new branch only.
    await expect(createWorktree(host, list, 'main-copy', '--detach')).rejects.toThrow(/not a commit/)
    sh(repo, 'branch', 'taken')
    await expect(createWorktree(host, list, 'taken', one)).rejects.toThrow(/exists already/)
  })

  it('prunes the worktrees whose folder is gone', async () => {
    commit(repo, 'a.txt', 'a\n', 'one')
    sh(repo, 'worktree', 'add', '-q', '-b', 'gone', join(dir, 'repo.gone'))
    renameSync(join(dir, 'repo.gone'), join(dir, 'elsewhere'))
    const list = await listWorktrees(host, repo)
    expect(list.worktrees.find((wt) => wt.branch === 'gone')?.prunable).toBe(true)
    expect(await pruneWorktrees(host, list)).toBe(1)
    expect((await listWorktrees(host, repo)).worktrees.map((wt) => wt.branch)).toEqual(['main'])
  })

  it('makes, renames, updates and pushes a branch without checking it out', async () => {
    commit(repo, 'a.txt', 'a\n', 'one')
    const one = sh(repo, 'rev-parse', 'HEAD')
    const origin = join(dir, 'origin.git')
    sh(dir, 'clone', '-q', '--bare', repo, origin)
    sh(repo, 'remote', 'add', 'origin', origin)
    sh(repo, 'fetch', '-q', 'origin')
    sh(repo, 'branch', '--set-upstream-to=origin/main', 'main')

    let list = await listWorktrees(host, repo)
    expect(await createBranch(host, list, ' side ', one)).toBe('side')
    expect(sh(repo, 'rev-parse', 'side')).toBe(one)
    expect(sh(repo, 'symbolic-ref', '--short', 'HEAD')).toBe('main')
    await expect(createBranch(host, list, '@{-1}', one)).rejects.toThrow(/plain branch name/)
    await expect(createBranch(host, list, 'side', one)).rejects.toThrow(/already exists/)
    expect(await renameBranch(host, list, 'side', 'renamed')).toBe('renamed')
    expect(sh(repo, 'rev-parse', 'renamed')).toBe(one)
    await expect(renameBranch(host, { ...list, defaultBranch: 'main' }, 'main', 'trunk')).rejects.toThrow(
      /default branch/,
    )

    // Push: a branch with no upstream sets one on origin.
    commit(repo, 'b.txt', 'b\n', 'two')
    sh(repo, 'branch', 'feature')
    list = await listWorktrees(host, repo)
    const target = await pushTarget(host, list, 'feature')
    expect(target).toMatchObject({ remote: 'origin', ref: 'refs/heads/feature', setUpstream: true })
    await pushBranch(host, list, 'feature', target)
    expect(sh(origin, 'rev-parse', 'feature')).toBe(sh(repo, 'rev-parse', 'feature'))
    expect(sh(repo, 'rev-parse', '--abbrev-ref', 'feature@{upstream}')).toBe('origin/feature')
    expect(await pushTarget(host, list, 'main')).toMatchObject({ name: 'origin/main', setUpstream: false, ahead: 1 })

    // Update: main checked out here fast-forwards to what origin gained.
    const other = join(dir, 'other')
    sh(dir, 'clone', '-q', origin, other)
    sh(other, 'switch', '-q', 'feature')
    commit(other, 'c.txt', 'c\n', 'from elsewhere')
    sh(other, 'push', '-q', 'origin', 'HEAD:refs/heads/feature')
    sh(repo, 'branch', '--set-upstream-to=origin/feature', 'renamed')
    expect(await updateBranch(host, list, 'feature')).toMatch(/^updated feature to origin\/feature/)
    expect(sh(repo, 'rev-parse', 'feature')).toBe(sh(other, 'rev-parse', 'HEAD'))
    expect(await updateBranch(host, list, 'feature')).toMatch(/up to date/)
    // Diverged: left alone.
    commit(repo, 'd.txt', 'd\n', 'mine')
    await expect(updateBranch(host, list, 'main')).resolves.toMatch(/everything origin\/main has/)
    sh(other, 'push', '-q', 'origin', 'HEAD:refs/heads/main', '--force')
    await expect(updateBranch(host, list, 'main')).rejects.toThrow(/diverged/)
    // Checked out nowhere, a branch behind its upstream moves all the same.
    expect(await updateBranch(host, list, 'renamed')).toMatch(/^updated renamed to origin\/feature/)
  })
})

describe("revert a commit's changes", () => {
  it('reverses the chosen files in the working tree, and refuses edited ones whole', async () => {
    mkdirSync(join(repo, 'sub'))
    commit(repo, 'a.txt', 'one\n', 'first')
    commit(repo, 'sub/b.txt', 'b\n', 'second')
    writeFileSync(join(repo, 'a.txt'), 'two\n')
    writeFileSync(join(repo, 'c.txt'), 'c\n')
    sh(repo, 'add', '-A')
    sh(repo, 'commit', '-q', '-m', 'third')
    const third = sh(repo, 'rev-parse', 'HEAD')
    // From a subfolder, one file of three: the other stays, nothing is staged.
    await gitRevertChanges(host, join(repo, 'sub'), third, ['a.txt'])
    expect(readFileSync(join(repo, 'a.txt'), 'utf8')).toBe('one\n')
    expect(existsSync(join(repo, 'c.txt'))).toBe(true)
    expect(sh(repo, 'diff', '--cached', '--name-only')).toBe('')
    // An added file goes.
    await gitRevertChanges(host, repo, third, ['c.txt'])
    expect(existsSync(join(repo, 'c.txt'))).toBe(false)
    // Edited since: refused, and neither file moves.
    sh(repo, 'checkout', '-q', '--', '.')
    writeFileSync(join(repo, 'a.txt'), 'mine\n')
    await expect(gitRevertChanges(host, repo, third, ['a.txt', 'c.txt'])).rejects.toThrow(/a\.txt/)
    expect(readFileSync(join(repo, 'a.txt'), 'utf8')).toBe('mine\n')
    expect(existsSync(join(repo, 'c.txt'))).toBe(true)
    // An older commit's file, from its own diff.
    sh(repo, 'checkout', '-q', '--', '.')
    await gitRevertChanges(host, repo, sh(repo, 'rev-parse', 'HEAD~1'), ['sub/b.txt'])
    expect(existsSync(join(repo, 'sub/b.txt'))).toBe(false)
  })
})
