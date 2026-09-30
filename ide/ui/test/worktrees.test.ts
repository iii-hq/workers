/* The worktree verbs against a real git, in throwaway repositories. This
   lives outside `src` because it drives git through node APIs, and the
   page's tsconfig (DOM only) type-checks `src`. */

import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, realpathSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  branchStanding,
  checkRemovable,
  createWorktree,
  deleteBranch,
  groupBranches,
  listWorktrees,
  mergeBranch,
  mergeWorktree,
  removeWorktree,
  switchPath,
  type WorktreeList,
} from '../src/page/worktrees'
import { commit, env, host, setAfterGit, sh } from './git-host'

let dir: string
let repo: string

async function withBranch(branch: string): Promise<{ list: WorktreeList; path: string }> {
  const wt = await createWorktree(host, await listWorktrees(host, repo), branch)
  return { list: await listWorktrees(host, wt.path), path: wt.path }
}

beforeEach(() => {
  dir = realpathSync(mkdtempSync(join(tmpdir(), 'ide-worktrees-')))
  env.HOME = dir
  env.XDG_CONFIG_HOME = dir
  setAfterGit(null)
  repo = join(dir, 'repo')
  mkdirSync(join(repo, 'sub'), { recursive: true })
  sh(dir, 'init', '-q', '-b', 'main', repo)
  writeFileSync(join(repo, 'sub', 'keep'), '')
  sh(repo, 'add', 'sub/keep')
  commit(repo, 'base.txt', 'base\n', 'base')
})

afterEach(() => rmSync(dir, { recursive: true, force: true }))

describe('worktrees', () => {
  it('creates <repo>.<branch>, lists it dirty and ahead, and switches into the same subfolder', async () => {
    const { path } = await withBranch('feat/a')
    expect(path).toBe(`${repo}.feat-a`)
    commit(path, 'a.txt', 'a\n', 'a')
    writeFileSync(join(path, 'scratch.txt'), 'wip\n')

    const list = await listWorktrees(host, join(repo, 'sub'))
    expect(list.defaultBranch).toBe('main')
    expect(list.current?.path).toBe(repo)
    const feat = list.worktrees.find((wt) => wt.branch === 'feat/a')
    expect(feat).toMatchObject({ path, main: false, dirty: true, ahead: 1, behind: 0 })
    expect(await switchPath(host, list, feat!, join(repo, 'sub'))).toBe(join(path, 'sub'))

    // An existing branch gets its worktree back instead of a second one.
    expect((await createWorktree(host, list, 'feat/a')).path).toBe(path)
    // `feat-a` maps onto the same folder: refused before git makes a branch.
    await expect(createWorktree(host, list, 'feat-a')).rejects.toThrow(/already exists/)
    expect(sh(repo, 'branch', '--list', 'feat-a')).toBe('')
  })

  it('lists every local branch, the most recently committed first, with its counts', async () => {
    sh(repo, 'branch', 'idle')
    env.GIT_COMMITTER_DATE = '2030-01-01T00:00:00Z'
    sh(repo, 'switch', '-q', '-c', 'fresh')
    commit(repo, 'f.txt', 'f\n', 'fresh')
    delete env.GIT_COMMITTER_DATE
    sh(repo, 'switch', '-q', 'main')

    const list = await listWorktrees(host, repo)
    expect(list.branches[0]).toEqual({ name: 'fresh', ahead: 1, behind: 0 })
    expect(list.branches.map((branch) => branch.name).sort()).toEqual(['fresh', 'idle', 'main'])
    // Only main is checked out: the others are branches without a worktree.
    expect(list.worktrees.map((wt) => wt.branch)).toEqual(['main'])
  })

  it('keeps a branch with its worktree while a rebase stopped there holds it', async () => {
    const { path } = await withBranch('feat/r')
    commit(path, 'base.txt', 'feat\n', 'feat side')
    commit(repo, 'base.txt', 'main\n', 'main side')
    // Stops on the conflict: git lists the worktree as detached meanwhile.
    spawnSync('git', ['rebase', 'main'], { cwd: path, env, encoding: 'utf8' })

    const list = await listWorktrees(host, repo)
    const row = list.worktrees.find((wt) => wt.path === path)
    expect(row).toMatchObject({ branch: null, held: { branch: 'feat/r', by: 'rebase' } })
    // Asking for the branch gives that worktree back instead of a second one.
    expect((await createWorktree(host, list, 'feat/r')).path).toBe(path)
  })

  it('lists a submodule by its checkout, not its git dir, and makes no worktrees of it', async () => {
    const lib = join(dir, 'lib')
    sh(dir, 'init', '-q', '-b', 'main', lib)
    commit(lib, 'lib.txt', 'lib\n', 'lib')
    sh(repo, '-c', 'protocol.file.allow=always', 'submodule', 'add', '-q', lib, 'mod')
    const mod = join(repo, 'mod')

    const list = await listWorktrees(host, mod)
    // Git names `<repo>/.git/modules/mod` as the main worktree.
    expect(list.worktrees[0]).toMatchObject({ path: mod, main: true, submodule: true })
    expect(list.current?.path).toBe(mod)
    await expect(createWorktree(host, list, 'topic')).rejects.toThrow(/submodule/)
  })

  it('deletes a branch no worktree has, and only from where it was when the user was asked', async () => {
    sh(repo, 'branch', 'merged-one')
    sh(repo, 'switch', '-q', '-c', 'side')
    commit(repo, 's.txt', 's\n', 'side')
    sh(repo, 'switch', '-q', 'main')
    const list = await listWorktrees(host, repo)

    const merged = await branchStanding(host, list, 'merged-one')
    expect(merged.unmerged).toBe(0)
    const side = await branchStanding(host, list, 'side')
    expect(side.unmerged).toBe(1)
    await deleteBranch(host, list, 'merged-one', merged.tip)
    expect(sh(repo, 'branch', '--list', 'merged-one')).toBe('')
    // It moved after the user was asked: that is not what they agreed to lose.
    sh(repo, 'branch', '-f', 'side', 'main')
    await expect(deleteBranch(host, list, 'side', side.tip)).rejects.toThrow(/moved/)
    expect(sh(repo, 'branch', '--list', 'side')).not.toBe('')
  })

  it('merges a branch no worktree has, squashed or replayed, moving main and its checkout', async () => {
    sh(repo, 'switch', '-q', '-c', 'sq')
    commit(repo, 'q1.txt', '1\n', 'q one')
    commit(repo, 'q2.txt', '2\n', 'q two')
    sh(repo, 'switch', '-q', 'main')
    commit(repo, 'm.txt', 'm\n', 'main moves')

    let list = await listWorktrees(host, repo)
    const squashed = await mergeBranch(host, list, 'sq', { squash: true, message: 'squash sq' })
    expect(squashed.target).toBe('main')
    expect(sh(repo, 'rev-parse', 'main')).toBe(squashed.sha)
    expect(sh(repo, 'log', '-1', '--format=%s', 'main')).toBe('squash sq')
    // main is checked out in the main worktree: its files moved with it.
    expect(existsSync(join(repo, 'q2.txt'))).toBe(true)
    expect(sh(repo, 'rev-parse', 'sq')).toBe(squashed.tip)

    sh(repo, 'switch', '-q', '-c', 'rp')
    commit(repo, 'r1.txt', '1\n', 'r one')
    sh(repo, 'switch', '-q', 'main')
    commit(repo, 'm2.txt', 'm2\n', 'main again')
    list = await listWorktrees(host, repo)
    await mergeBranch(host, list, 'rp', { squash: false, message: '' })
    expect(sh(repo, 'log', '-2', '--format=%s', 'main').split('\n')).toEqual(['r one', 'main again'])
    expect(existsSync(join(repo, 'r1.txt'))).toBe(true)

    // A conflict changes nothing.
    sh(repo, 'switch', '-q', '-c', 'clash')
    commit(repo, 'base.txt', 'clash\n', 'clash')
    sh(repo, 'switch', '-q', 'main')
    commit(repo, 'base.txt', 'main\n', 'main clash')
    list = await listWorktrees(host, repo)
    const before = sh(repo, 'rev-parse', 'main')
    await expect(mergeBranch(host, list, 'clash', { squash: true, message: 'x' })).rejects.toThrow(
      /conflicts with main in base\.txt/,
    )
    await expect(mergeBranch(host, list, 'clash', { squash: false, message: '' })).rejects.toThrow(/conflict/)
    expect(sh(repo, 'rev-parse', 'main')).toBe(before)
  })

  it('merges with squash, uncommitted work included, then removes the worktree and its branch', async () => {
    const { list, path } = await withBranch('feat/b')
    commit(path, 'b1.txt', '1\n', 'b one')
    commit(path, 'b2.txt', '2\n', 'b two')
    writeFileSync(join(path, 'b3.txt'), '3\n')
    const wt = list.worktrees.find((entry) => entry.path === path)!

    const { target } = await mergeWorktree(host, list, wt, { squash: true, message: 'feat: b' })
    expect(target).toBe('main')
    expect(sh(repo, 'log', '--format=%s', 'main')).toBe('feat: b\nbase')
    expect(['b1.txt', 'b2.txt', 'b3.txt'].every((file) => existsSync(join(repo, file)))).toBe(true)

    expect(await removeWorktree(host, list, wt)).toEqual({ branchDeleted: true })
    expect(existsSync(path)).toBe(false)
    expect(sh(repo, 'branch', '--list', 'feat/b')).toBe('')
  })

  it('aborts a conflicting rebase and leaves the branch checked out', async () => {
    const { list, path } = await withBranch('feat/c')
    commit(path, 'base.txt', 'theirs\n', 'c')
    commit(repo, 'base.txt', 'ours\n', 'main moves')
    const wt = list.worktrees.find((entry) => entry.path === path)!

    await expect(mergeWorktree(host, list, wt, { squash: false, message: '' })).rejects.toThrow(
      /conflicts in base\.txt/,
    )
    expect(sh(path, 'symbolic-ref', 'HEAD')).toBe('refs/heads/feat/c')
    expect(sh(repo, 'log', '-1', '--format=%s', 'main')).toBe('main moves')

    // An abort that fails as well, here on a killed git's index.lock, is
    // reported instead of passed off as a clean stop.
    setAfterGit((args, cwd) => {
      if (args.join(' ') === 'rebase refs/heads/main') {
        writeFileSync(join(sh(cwd, 'rev-parse', '--absolute-git-dir'), 'index.lock'), '')
      }
    })
    await expect(mergeWorktree(host, list, wt, { squash: false, message: '' })).rejects.toThrow(
      /conflicts in base\.txt \(git rebase --abort: .*\); a rebase in progress or an index\.lock may be left in/,
    )
  })

  it('deletes a squash-merged branch on remove but keeps an unmerged one', async () => {
    const merged = await withBranch('feat/d')
    commit(merged.path, 'd.txt', 'd\n', 'd')
    commit(repo, 'd.txt', 'd\n', 'd, squash-merged')
    const unmerged = await withBranch('feat/e')
    commit(unmerged.path, 'e.txt', 'e\n', 'e')
    const list = await listWorktrees(host, repo)

    const d = list.worktrees.find((wt) => wt.branch === 'feat/d')!
    const e = list.worktrees.find((wt) => wt.branch === 'feat/e')!
    expect(await removeWorktree(host, list, d)).toEqual({ branchDeleted: true })
    expect(await removeWorktree(host, list, e)).toEqual({ branchDeleted: false })
    expect(sh(repo, 'branch', '--list', 'feat/e')).toContain('feat/e')
    await expect(removeWorktree(host, list, list.worktrees[0])).rejects.toThrow(/main worktree/)
  })

  it('refuses to remove a worktree that holds another worktree, or a locked one', async () => {
    commit(repo, '.gitignore', '.worktrees/\n', 'ignore .worktrees')
    const { path: parent } = await withBranch('feat/parent')
    const child = join(parent, '.worktrees', 'child')
    sh(parent, 'worktree', 'add', '-q', '-b', 'feat/child', child)
    writeFileSync(join(child, 'base.txt'), 'child wip\n')
    const list = await listWorktrees(host, child)
    const row = list.worktrees.find((wt) => wt.path === parent)!
    expect(row.dirty).toBe(false)

    await expect(removeWorktree(host, list, row, true)).rejects.toThrow(/would delete it too/)
    expect(existsSync(join(child, 'base.txt'))).toBe(true)
    await expect(checkRemovable(host, list, row, true)).rejects.toThrow(
      '.worktrees/child (feat/child) is a worktree inside repo.feat-parent',
    )

    // Locked after the list was read.
    sh(repo, 'worktree', 'lock', child)
    const locked = list.worktrees.find((wt) => wt.path === child)!
    await expect(removeWorktree(host, list, locked, true)).rejects.toThrow(/git worktree unlock/)
    expect(existsSync(child)).toBe(true)
  })

  it('asks for force before dropping commits only a detached HEAD holds, or submodules', async () => {
    const { list, path } = await withBranch('feat/det')
    sh(path, 'checkout', '-q', '--detach')
    commit(path, 'lone.txt', 'lone\n', 'lone')
    const wt = list.worktrees.find((entry) => entry.path === path)!
    await expect(checkRemovable(host, list, wt, false)).rejects.toMatchObject({
      dirty: true,
      message: expect.stringMatching(/1 commit on a detached HEAD that no branch holds/),
    })
    sh(path, 'branch', 'keep-lone')
    await expect(checkRemovable(host, list, wt, false)).resolves.toBeUndefined()

    const lib = join(dir, 'lib')
    sh(dir, 'init', '-q', '-b', 'main', lib)
    commit(lib, 'lib.txt', 'lib\n', 'lib')
    const sub = await withBranch('feat/sub')
    sh(sub.path, '-c', 'protocol.file.allow=always', 'submodule', 'add', '-q', lib, 'vendor/lib')
    sh(sub.path, 'commit', '-q', '-m', 'add lib')
    const subWt = sub.list.worktrees.find((entry) => entry.path === sub.path)!
    await expect(checkRemovable(host, sub.list, subWt, false)).rejects.toMatchObject({
      dirty: true,
      message: expect.stringMatching(/has submodule repositories$/),
    })
    // A deinit leaves the submodule's repository in the worktree's git dir,
    // which git still refuses to delete without force.
    sh(sub.path, 'submodule', 'deinit', '-q', '-f', 'vendor/lib')
    await expect(checkRemovable(host, sub.list, subWt, false)).rejects.toMatchObject({ dirty: true })
    // Everything that would go is named at once.
    writeFileSync(join(sub.path, 'wip.txt'), 'wip\n')
    await expect(checkRemovable(host, sub.list, subWt, false)).rejects.toThrow(
      /has uncommitted changes and submodule repositories$/,
    )
    await removeWorktree(host, sub.list, subWt, true)
    expect(existsSync(sub.path)).toBe(false)

    // A nested repository added as a bare gitlink, with no .gitmodules entry.
    const { list: embeddedList, path: embedded } = await withBranch('feat/embedded')
    sh(embedded, 'init', '-q', '-b', 'main', 'tool')
    commit(join(embedded, 'tool'), 'tool.txt', 'tool\n', 'tool')
    sh(embedded, 'add', 'tool')
    sh(embedded, 'commit', '-q', '-m', 'add tool')
    const embeddedWt = embeddedList.worktrees.find((entry) => entry.path === embedded)!
    await expect(checkRemovable(host, embeddedList, embeddedWt, false)).rejects.toThrow(/has submodule repositories$/)
  })

  it('puts the branch back when a hook rejects the squash commit', async () => {
    const { list, path } = await withBranch('feat/h')
    commit(path, 'h1.txt', '1\n', 'h one')
    commit(path, 'h2.txt', '2\n', 'h two')
    const tip = sh(repo, 'rev-parse', 'feat/h')
    mkdirSync(join(repo, '.git', 'hooks'), { recursive: true })
    writeFileSync(join(repo, '.git', 'hooks', 'pre-commit'), '#!/bin/sh\necho "lint failed" >&2\nexit 1\n')
    chmodSync(join(repo, '.git', 'hooks', 'pre-commit'), 0o755)
    const wt = list.worktrees.find((entry) => entry.path === path)!

    await expect(mergeWorktree(host, list, wt, { squash: true, message: 'feat: h' })).rejects.toThrow(/lint failed/)
    expect(sh(repo, 'rev-parse', 'feat/h')).toBe(tip)
    expect(sh(repo, 'log', '-1', '--format=%s', 'main')).toBe('base')
  })

  it('fast-forwards main where it is checked out now, not where the list saw it', async () => {
    const { list, path } = await withBranch('feat/x')
    commit(path, 'x.txt', 'x\n', 'x')
    const wt = list.worktrees.find((entry) => entry.path === path)!
    sh(repo, 'switch', '-q', '-c', 'fix/y')

    await mergeWorktree(host, list, wt, { squash: true, message: 'feat: x' })
    expect(sh(repo, 'log', '--format=%s', 'main')).toBe('feat: x\nbase')
    expect(sh(repo, 'log', '--format=%s', 'fix/y')).toBe('base')
  })

  it('refuses when the checkout of main switches branch during the merge', async () => {
    const { list, path } = await withBranch('feat/z')
    commit(path, 'z.txt', 'z\n', 'z')
    const wt = list.worktrees.find((entry) => entry.path === path)!
    // Right after the merge has looked up where main is checked out.
    setAfterGit((args) => {
      if (args[0] !== 'worktree') return
      setAfterGit(null)
      sh(repo, 'switch', '-q', '-c', 'fix/z')
    })

    await expect(mergeWorktree(host, list, wt, { squash: false, message: '' })).rejects.toThrow(
      /main is no longer checked out in repo/,
    )
    expect(sh(repo, 'log', '--format=%s', 'main')).toBe('base')
    expect(sh(repo, 'log', '--format=%s', 'fix/z')).toBe('base')
  })

  it('moves a main checked out nowhere, by fast-forward only', async () => {
    sh(repo, 'switch', '-q', '-c', 'dev')
    const a = await withBranch('feat/a')
    expect(a.list.defaultBranch).toBe('main')
    // `@{-1}` names main here; a failed create must not touch main.
    await expect(createWorktree(host, a.list, '@{-1}')).rejects.toThrow(/not a plain branch name/)
    commit(a.path, 'a.txt', 'a\n', 'a')
    const rowA = a.list.worktrees.find((entry) => entry.path === a.path)!
    await mergeWorktree(host, a.list, rowA, { squash: false, message: '' })
    expect(sh(repo, 'log', '--format=%s', 'main')).toBe('a\nbase')

    const b = await withBranch('feat/b')
    commit(b.path, 'b.txt', 'b\n', 'b')
    const rowB = b.list.worktrees.find((entry) => entry.path === b.path)!
    // main moves on elsewhere while this merge rebases.
    setAfterGit((args) => {
      if (args[0] !== 'rebase') return
      setAfterGit(null)
      sh(repo, 'update-ref', 'refs/heads/main', sh(repo, 'commit-tree', '-p', 'main', '-m', 'elsewhere', 'main^{tree}'))
    })
    await expect(mergeWorktree(host, b.list, rowB, { squash: false, message: '' })).rejects.toThrow(/non-fast-forward/)
    expect(sh(repo, 'log', '--format=%s', 'main')).toBe('elsewhere\na\nbase')
  })

  it('counts untracked files as changes even where git status hides them', async () => {
    sh(repo, 'config', 'status.showUntrackedFiles', 'no')
    const { path } = await withBranch('feat/u')
    commit(path, 'u.txt', 'u\n', 'u')
    writeFileSync(join(path, 'notes.md'), 'hours of work\n')
    const list = await listWorktrees(host, repo)
    const wt = list.worktrees.find((entry) => entry.path === path)!
    expect(wt.dirty).toBe(true)

    await expect(mergeWorktree(host, list, wt, { squash: false, message: '' })).rejects.toThrow(
      /commit the changes first/,
    )
    await expect(removeWorktree(host, list, wt)).rejects.toMatchObject({ dirty: true })
    expect(existsSync(join(path, 'notes.md'))).toBe(true)

    // A file made after that check still stops git's own removal.
    rmSync(join(path, 'notes.md'))
    setAfterGit((args) => {
      if (args.includes('status')) writeFileSync(join(path, 'late.md'), 'late\n')
    })
    await expect(removeWorktree(host, list, wt)).rejects.toThrow(/untracked/)
    expect(existsSync(join(path, 'late.md'))).toBe(true)
  })

  it('keeps a branch whose changes a merge driver drops', async () => {
    commit(repo, '.gitattributes', 'CHANGELOG.md merge=ours\n', 'attributes')
    commit(repo, 'CHANGELOG.md', '# changes\n', 'changelog')
    sh(repo, 'config', 'merge.ours.driver', 'true')
    const notes = await withBranch('docs/notes')
    commit(notes.path, 'CHANGELOG.md', '# changes\n- 1.2.0 notes\n', 'notes')
    commit(repo, 'CHANGELOG.md', '# changes\n- 1.1.1 fix\n', 'fix')
    let list = await listWorktrees(host, repo)
    const row = list.worktrees.find((entry) => entry.path === notes.path)!
    expect(await removeWorktree(host, list, row)).toEqual({ branchDeleted: false })
    expect(sh(repo, 'branch', '--list', 'docs/notes')).toContain('docs/notes')

    // The same driver named by `merge.default`, for a path without the attribute.
    sh(repo, 'config', 'merge.default', 'ours')
    const other = await withBranch('docs/other')
    commit(other.path, 'base.txt', 'theirs\n', 'theirs')
    commit(repo, 'base.txt', 'ours\n', 'ours')
    list = await listWorktrees(host, repo)
    const otherRow = list.worktrees.find((entry) => entry.path === other.path)!
    expect(await removeWorktree(host, list, otherRow)).toEqual({ branchDeleted: false })
  })

  it('drops only the gone worktree it is asked to', async () => {
    const gone = await withBranch('stale/a')
    const away = await withBranch('usb/b')
    rmSync(gone.path, { recursive: true })
    renameSync(away.path, join(dir, 'unplugged'))
    const list = await listWorktrees(host, repo)
    const row = list.worktrees.find((entry) => entry.path === gone.path)!
    expect(row.prunable).toBe(true)

    expect(await removeWorktree(host, list, row)).toEqual({ branchDeleted: true })
    renameSync(join(dir, 'unplugged'), away.path)
    expect((await listWorktrees(host, repo)).worktrees.map((wt) => wt.path)).toEqual([repo, away.path])
    expect(sh(away.path, 'status', '--porcelain')).toBe('')
    // The list read while it was away: its branch gets no second worktree.
    await expect(createWorktree(host, list, 'usb/b')).rejects.toThrow(/whose folder is gone/)
  })

  it('lists a bare repository as the main entry and creates beside it', async () => {
    const bare = join(dir, 'proj.git')
    sh(dir, 'clone', '-q', '--bare', repo, bare)
    sh(bare, 'worktree', 'add', '-q', join(dir, 'proj.main'), 'main')
    const list = await listWorktrees(host, join(dir, 'proj.main'))
    expect(list.worktrees[0]).toMatchObject({ path: bare, main: true, bare: true, branch: null })
    expect(list.defaultBranch).toBe('main')
    expect(list.current?.path).toBe(join(dir, 'proj.main'))

    expect((await createWorktree(host, list, 'feat/b')).path).toBe(join(dir, 'proj.feat-b'))
    await expect(removeWorktree(host, list, list.worktrees[0])).rejects.toThrow(/main worktree/)
  })
})

describe('groupBranches', () => {
  it('groups by the first prefix, keeps a lone prefix flat, and orders by the newest branch', () => {
    const names = ['feat/a', 'main', 'fix/x', 'feat/b', 'scratch/one', 'fix/y', 'andersonleal/feat/c', 'andersonleal/d']
    expect(
      groupBranches(names.map((name) => ({ name }))).map((g) => [g.prefix, g.branches.map((b) => b.name)]),
    ).toEqual([
      ['feat', ['feat/a', 'feat/b']],
      [null, ['main']],
      ['fix', ['fix/x', 'fix/y']],
      [null, ['scratch/one']],
      ['andersonleal', ['andersonleal/feat/c', 'andersonleal/d']],
    ])
  })
})
