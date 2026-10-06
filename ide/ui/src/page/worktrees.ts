/* Worktree verbs in the style of worktrunk (github.com/max-sixty/worktrunk),
   over plain git:
   - list the repository's worktrees, with whether each is dirty and how far
     ahead of / behind the default branch it is, and its local branches;
   - create one as a `<repo>.<branch>` folder beside the main worktree;
   - remove one, deleting its branch only once the default branch has it;
   - merge one into the default branch (squash, rebase, fast-forward).
   Everything goes through `shell::exec` in argv form, like git-actions.
   The verbs that change things re-read git's worktree list and status
   instead of trusting the list the view loaded, which can be stale. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { parseRecentBranches, readRecentReflog, readRemoteBranches, readTags } from './branch-actions'
import { coderReadFiles, joinPath, workspaceValidate } from './coder'
import { git, run } from './git-actions'
import { basename } from './paths'

// ponytail: adding or removing a tree, and rebasing, run in the foreground
// under the worker's cap (max_timeout_ms, 120 s). Move them to
// shell::exec_bg if worktrees carry build folders too big to delete in that.
const TREE_TIMEOUT_MS = 120_000

/** `git status` that lists untracked files even where the repository sets
    `status.showUntrackedFiles=no` (they go with the folder all the same), and
    reads without taking index.lock, which a git writing there at that moment
    would otherwise fail on. */
const STATUS = ['--no-optional-locks', 'status', '--porcelain', '--untracked-files=normal']

export interface Worktree {
  path: string
  /** `null` when HEAD is detached, which is also the case mid-rebase. */
  branch: string | null
  /** The repository's main worktree: the first `git worktree list` entry. */
  main: boolean
  /** A submodule's main worktree. Git names its git dir, inside the
      superproject's `.git`; `path` is the checkout that git dir serves. */
  submodule?: boolean
  /** A bare repository's own folder, listed as its main worktree. Nothing
      is checked out there. */
  bare: boolean
  locked: boolean
  /** Its folder is gone; removing it only drops git's entry. */
  prunable: boolean
  /** The commit checked out there. Absent for a bare repository. */
  head?: string
  /** Uncommitted changes, untracked files included. Absent when unknown. */
  dirty?: boolean
  /** Commits ahead of / behind the default branch. Absent when unknown. */
  ahead?: number
  behind?: number
  /** The branch a rebase or bisect stopped here returns to. Git lists the
      worktree as detached meanwhile, yet keeps the branch as its own. */
  held?: { branch: string; by: 'rebase' | 'bisect' }
  /** The subject and the committer date (unix seconds) of `head`. */
  tip?: { subject: string; date: number }
}

/** The branch checked out in `wt`, or held there by a rebase or bisect. */
export function branchOf(wt: Worktree): string | null {
  return wt.branch ?? wt.held?.branch ?? null
}

/** A local branch. */
export interface Branch {
  name: string
  /** Commits ahead of / behind the default branch. Absent when unknown. */
  ahead?: number
  behind?: number
  /** The remote-tracking branch it follows (`origin/main`), when it has one. */
  upstream?: string
}

/** The branches under one `prefix/`, or a lone branch (`prefix` null). */
export interface BranchGroup<T> {
  prefix: string | null
  branches: T[]
}

/** Branches grouped by the part of their name before the first `/`, one
    level deep, like an IDE's branch tree. A prefix only one branch has
    stays a lone branch. Groups and lone branches keep the order of their
    first branch. */
export function groupBranches<T extends { name: string }>(branches: readonly T[]): BranchGroup<T>[] {
  const prefixOf = (name: string) => (name.indexOf('/') > 0 ? name.slice(0, name.indexOf('/')) : null)
  const byPrefix = new Map<string, T[]>()
  for (const branch of branches) {
    const prefix = prefixOf(branch.name)
    if (prefix === null) continue
    const members = byPrefix.get(prefix)
    if (members) members.push(branch)
    else byPrefix.set(prefix, [branch])
  }
  const groups: BranchGroup<T>[] = []
  for (const branch of branches) {
    const prefix = prefixOf(branch.name)
    const members = prefix === null ? undefined : byPrefix.get(prefix)
    if (prefix === null || members === undefined || members.length < 2) {
      groups.push({ prefix: null, branches: [branch] })
    } else if (members[0] === branch) {
      groups.push({ prefix, branches: members })
    }
  }
  return groups
}

export interface WorktreeList {
  worktrees: Worktree[]
  /** Every local branch, the most recently committed first. */
  branches: Branch[]
  /** What ahead/behind count against and what merges land on. */
  defaultBranch: string | null
  /** The worktree holding the browsed folder. */
  current: Worktree | null
  /** Remote-tracking branches (`origin/main`), the most recently committed first. */
  remotes: string[]
  /** Local branches checked out lately in the browsed folder, newest first. */
  recent: string[]
  /** Tags, the newest first. */
  tags: string[]
}

/** `git worktree list --porcelain -z`: NUL-terminated attribute lines, with
    an empty field closing each record. */
export function parseWorktreeList(stdout: string): Worktree[] {
  const worktrees: Worktree[] = []
  let entry: Worktree | null = null
  for (const field of stdout.split('\0')) {
    if (field === '') {
      entry = null
      continue
    }
    const space = field.indexOf(' ')
    const key = space === -1 ? field : field.slice(0, space)
    const value = space === -1 ? '' : field.slice(space + 1)
    if (key === 'worktree') {
      entry = { path: value, branch: null, main: worktrees.length === 0, bare: false, locked: false, prunable: false }
      worktrees.push(entry)
    } else if (entry !== null) {
      if (key === 'branch') entry.branch = value.replace(/^refs\/heads\//, '')
      else if (key === 'HEAD') entry.head = value
      else if (key === 'bare') entry.bare = true
      else if (key === 'locked') entry.locked = true
      else if (key === 'prunable') entry.prunable = true
    }
  }
  return worktrees
}

/** `path` is `dir` itself or lies somewhere inside it. */
export function isInside(path: string, dir: string): boolean {
  return path === dir || path.startsWith(`${dir}/`)
}

/** The worktree `path` sits in: the longest matching worktree path, since
    worktrees can nest inside one another's folders. */
export function worktreeAt(worktrees: readonly Worktree[], path: string): Worktree | null {
  let best: Worktree | null = null
  for (const wt of worktrees) {
    if (isInside(path, wt.path) && (best === null || wt.path.length > best.path.length)) best = wt
  }
  return best
}

/** Worktrunk's default layout: `<repo>.<branch>` beside the main worktree,
    with `/` and `\` in the branch turned into `-`. A bare repository's
    `<repo>.git` folder stands for `<repo>`. */
export function worktreePathFor(mainPath: string, branch: string): string {
  return `${mainPath.replace(/\.git$/, '')}.${branch.replace(/[/\\]/g, '-')}`
}

async function readWorktrees(host: Host, cwd: string): Promise<Worktree[]> {
  const out = await git(host, cwd, ['worktree', 'list', '--porcelain', '-z'])
  // `-z` came with git 2.36; an older git rejects it as a usage error.
  if (out.exit_code === 129) throw new Error('the Worktrees view needs git 2.36 or newer')
  if (out.exit_code !== 0) throw new Error(out.stderr.trim() || 'git worktree list failed')
  const worktrees = parseWorktreeList(out.stdout)
  const main = worktrees[0]
  if (main?.path.includes('/.git/')) {
    main.submodule = true
    const top = await git(host, main.path, ['rev-parse', '--show-toplevel']).catch(() => null)
    if (top?.exit_code === 0 && top.stdout.trim() !== '') main.path = top.stdout.trim()
  }
  return worktrees
}

/** `dirty: false` skips the dirty marks: a `git status` per worktree. */
export async function listWorktrees(host: Host, root: string, dirty = true): Promise<WorktreeList> {
  // Two independent chains: the worktrees, their dirty marks and what a
  // stopped rebase or bisect left in them, and the default branch and the
  // branches counted against it.
  const [[worktrees, held], [defaultBranch, branches], remotes, reflog, tags] = await Promise.all([
    readWorktrees(host, root).then(async (list) => {
      const [, , held] = await Promise.all([
        dirty ? fillDirty(host, list) : undefined,
        fillTips(host, root, list),
        readHeld(host, list),
      ])
      return [list, held] as const
    }),
    findDefaultBranch(host, root).then(async (target) => [target, await readBranches(host, root, target)] as const),
    readRemoteBranches(host, root),
    readRecentReflog(host, root),
    readTags(host, root),
  ])
  const byName = new Map(branches.map((branch) => [branch.name, branch]))
  // Only a local branch is held: a rebase of a detached HEAD names none.
  for (const { wt, rebased, bisected } of held) {
    if (byName.has(rebased)) wt.held = { branch: rebased, by: 'rebase' }
    else if (byName.has(bisected)) wt.held = { branch: bisected, by: 'bisect' }
  }
  for (const wt of worktrees) {
    const own = branchOf(wt)
    const branch = own === null ? undefined : byName.get(own)
    if (branch?.ahead !== undefined) [wt.ahead, wt.behind] = [branch.ahead, branch.behind]
  }
  const current = worktreeAt(worktrees, root)
  const recent = parseRecentBranches(
    reflog,
    branches.map((branch) => branch.name),
    current === null ? null : branchOf(current),
  )
  return { worktrees, branches, defaultBranch, current, remotes, recent, tags }
}

/** The first local branch among the one `origin/HEAD` names,
    `init.defaultBranch`, `main` and `master`. Never just what the main
    worktree has checked out: that can be any side branch, and merges
    would land on it. */
async function findDefaultBranch(host: Host, cwd: string): Promise<string | null> {
  // Which of them exist comes from the local branches, listed alongside
  // rather than after; the first in that order wins.
  const [remote, init, found] = await Promise.all([
    git(host, cwd, ['symbolic-ref', '--quiet', '--short', 'refs/remotes/origin/HEAD']),
    git(host, cwd, ['config', 'init.defaultBranch']),
    git(host, cwd, ['for-each-ref', '--format=%(refname)', 'refs/heads']),
  ])
  if (found.exit_code !== 0) return null
  const names = [
    remote.exit_code === 0 ? remote.stdout.trim().replace(/^origin\//, '') : '',
    init.exit_code === 0 ? init.stdout.trim() : '',
    'main',
    'master',
  ]
  const present = new Set(found.stdout.split('\n'))
  return names.find((name) => name !== '' && present.has(`refs/heads/${name}`)) ?? null
}

/** Every local branch in a single `for-each-ref`, the most recently
    committed first, with its counts against `target`. `%(ahead-behind:…)`
    needs git 2.41; with an older git the counts stay unknown. */
async function readBranches(host: Host, cwd: string, target: string | null): Promise<Branch[]> {
  const read = (counts: boolean) =>
    git(host, cwd, [
      'for-each-ref',
      '--sort=-committerdate',
      `--format=%(refname)%09%(upstream:short)${counts ? `%09%(ahead-behind:refs/heads/${target})` : ''}`,
      'refs/heads',
    ])
  let out = await read(target !== null)
  if (out.exit_code !== 0 && target !== null) out = await read(false)
  if (out.exit_code !== 0) return []
  return parseBranches(out.stdout)
}

/** `for-each-ref` lines of `refname<TAB>upstream[<TAB>ahead behind]`. */
export function parseBranches(stdout: string): Branch[] {
  const branches: Branch[] = []
  for (const line of stdout.split('\n')) {
    const [ref, upstream, counts] = line.split('\t')
    if (!ref?.startsWith('refs/heads/')) continue
    const branch: Branch = { name: ref.slice('refs/heads/'.length) }
    if (upstream) branch.upstream = upstream
    // Branch names hold no spaces.
    const [ahead, behind] = counts?.split(' ') ?? []
    if (ahead !== undefined && behind !== undefined) [branch.ahead, branch.behind] = [Number(ahead), Number(behind)]
    branches.push(branch)
  }
  return branches
}

/** For each detached worktree, the names a stopped rebase (its `head-name`)
    and bisect (`BISECT_START`) left in the worktree's git dir: read before
    the branches are, so the caller keeps only a name that is one. */
async function readHeld(
  host: Host,
  worktrees: readonly Worktree[],
): Promise<{ wt: Worktree; rebased: string; bisected: string }[]> {
  const detached = worktrees.filter((wt) => wt.branch === null && !wt.bare && !wt.prunable)
  const held = await Promise.all(
    detached.map(async (wt) => {
      const paths = await git(host, wt.path, [
        'rev-parse',
        '--path-format=absolute',
        '--git-path',
        'rebase-merge/head-name',
        '--git-path',
        'rebase-apply/head-name',
        '--git-path',
        'BISECT_START',
      ]).catch(() => null)
      if (paths?.exit_code !== 0) return null
      const [merge, apply, bisect] = paths.stdout.split('\n')
      const files = await coderReadFiles(host, [merge, apply, bisect]).catch(() => [])
      const read = (path: string) => files.find((file) => file.path === path && file.success)?.content?.trim() ?? ''
      return { wt, rebased: (read(merge) || read(apply)).replace(/^refs\/heads\//, ''), bisected: read(bisect) }
    }),
  )
  return held.filter((entry) => entry !== null)
}

/** Each worktree's last commit, for its row: one `git log` over every head. */
async function fillTips(host: Host, cwd: string, worktrees: readonly Worktree[]): Promise<void> {
  const heads = [...new Set(worktrees.flatMap((wt) => (wt.head ? [wt.head] : [])))]
  if (heads.length === 0) return
  const out = await git(host, cwd, ['log', '--no-walk=unsorted', '--format=%H%x00%ct%x00%s', ...heads]).catch(
    () => null,
  )
  if (out?.exit_code !== 0) return
  const tips = new Map<string, { subject: string; date: number }>()
  for (const line of out.stdout.split('\n')) {
    const [sha, date, subject] = line.split('\0')
    if (sha && subject !== undefined) tips.set(sha, { subject, date: Number(date) })
  }
  for (const wt of worktrees) {
    const tip = wt.head ? tips.get(wt.head) : undefined
    if (tip) wt.tip = tip
  }
}

async function fillDirty(host: Host, worktrees: readonly Worktree[]): Promise<void> {
  // ponytail: runs four `git status` at a time. Load status per row, lazily,
  // if a repository grows to hundreds of worktrees.
  const queue = worktrees.filter((wt) => !wt.prunable && !wt.bare)
  const drain = async () => {
    for (let wt = queue.shift(); wt !== undefined; wt = queue.shift()) {
      const out = await git(host, wt.path, STATUS).catch(() => null)
      if (out?.exit_code === 0) wt.dirty = out.stdout.trim() !== ''
    }
  }
  await Promise.all([drain(), drain(), drain(), drain()])
}

/** Where switching to `target` lands: in the same subfolder the browsed
    root is in, when `target` has that folder; otherwise at its top. */
export async function switchPath(host: Host, list: WorktreeList, target: Worktree, root: string): Promise<string> {
  const from = list.current
  const rel = from === null || root === from.path ? '' : root.slice(from.path.length + 1)
  if (rel === '') return target.path
  try {
    return (await workspaceValidate(host, joinPath(target.path, rel))).path
  } catch {
    return target.path
  }
}

/** `wt switch --create`: a worktree for `branch` at `<repo>.<branch>`. It
    checks out the branch when it exists, and otherwise creates the branch
    from `from`, or from the default branch. A branch that already has a
    worktree just gets that worktree back. Starting from a remote branch
    (`origin/x`) sets it as the new branch's upstream, as `git branch` does. */
export async function createWorktree(host: Host, list: WorktreeList, branch: string, from?: string): Promise<Worktree> {
  const name = branch.trim()
  // A branch a stopped rebase or bisect holds has its worktree already.
  const existing = list.worktrees.find((wt) => branchOf(wt) === name)
  if (existing?.prunable) {
    throw new Error(`${name} is still checked out at ${existing.path}, whose folder is gone; prune that worktree first`)
  }
  if (existing) return existing
  const main = list.worktrees[0]
  if (!main) throw new Error('not a git repository')
  // `<repo>.<branch>` beside a submodule would land in the superproject.
  if (main.submodule) throw new Error('worktrees of a submodule are not supported')
  // It expands `@{-1}` and the like into another branch's name: take only
  // names that stand for themselves.
  const valid = await run(host, main.path, ['check-ref-format', '--branch', name], 'git check-ref-format')
  if (valid.stdout.trim() !== name) throw new Error(`${name} is not a plain branch name`)
  const path = worktreePathFor(main.path, name)
  const taken = await workspaceValidate(host, path).then(
    () => true,
    () => false,
  )
  if (taken) throw new Error(`${path} already exists`)
  const known = await git(host, main.path, ['rev-parse', '--verify', '--quiet', `refs/heads/${name}`])
  // An existing branch keeps its own history: a start point only makes sense for a new one.
  if (from !== undefined && known.exit_code === 0) throw new Error(`${name} exists already; open it instead`)
  const start = from ?? list.defaultBranch ?? 'HEAD'
  if (from !== undefined) {
    const valid = await git(host, main.path, [
      'rev-parse',
      '--verify',
      '--quiet',
      '--end-of-options',
      `${from}^{commit}`,
    ])
    if (from.startsWith('-') || valid.exit_code !== 0) throw new Error(`${from} is not a commit`)
  }
  try {
    await run(
      host,
      main.path,
      known.exit_code === 0 ? ['worktree', 'add', path, name] : ['worktree', 'add', '-b', name, path, start],
      'git worktree add',
      TREE_TIMEOUT_MS,
    )
  } catch (err) {
    // `-b` makes the branch before the folder, so a failed add leaves it
    // behind. `-d` takes it back, and keeps one that already holds commits.
    if (known.exit_code !== 0) await git(host, main.path, ['branch', '-d', name])
    throw err
  }
  return { path, branch: name, main: false, bare: false, locked: false, prunable: false, dirty: false }
}

/** Resolves when `wt` can be removed; otherwise throws why not, in a few
    words. Uncommitted or untracked files throw an error marked
    `dirty: true`, which only `force` gets past, so the caller can ask
    before they are deleted. Git's list and status are read afresh: a lock,
    a nested worktree or an edit can be newer than `list`. */
export async function checkRemovable(host: Host, list: WorktreeList, wt: Worktree, force: boolean): Promise<void> {
  const main = list.worktrees[0]
  if (!main || wt.main || wt.bare) throw new Error('the main worktree cannot be removed')
  const name = basename(wt.path)
  const worktrees = await readWorktrees(host, main.path)
  const now = worktrees.find((other) => other.path === wt.path)
  if (!now) throw new Error(`${name} is no longer a worktree`)
  if (now.locked) throw new Error(`${name} is locked; unlock it first with git worktree unlock ${wt.path}`)
  // `git worktree remove` deletes the whole folder, worktrees inside it too.
  const nested = worktrees.find((other) => other !== now && !other.prunable && isInside(other.path, wt.path))
  if (nested) {
    const where = nested.path.slice(wt.path.length + 1)
    throw new Error(
      `${where} (${nested.branch ?? 'detached'}) is a worktree inside ${name}; removing ${name} would delete it too`,
    )
  }
  if (force || now.prunable) return
  // Each of these is lost with the folder, and only --force takes them. They
  // are named together, so a forced removal is confirmed knowing all of it.
  const lost: string[] = []
  const status = await run(host, wt.path, STATUS, 'git status')
  if (status.stdout.trim() !== '') lost.push('uncommitted changes')
  // A detached HEAD (a detached worktree, or one mid-rebase) can hold commits
  // no branch has; the worktree's own HEAD reflog goes with it.
  const head = await git(host, wt.path, ['symbolic-ref', '-q', 'HEAD'])
  if (head.exit_code !== 0) {
    const lone = await git(host, wt.path, ['rev-list', '--count', 'HEAD', '--not', '--branches', '--tags', '--remotes'])
    const count = lone.exit_code === 0 ? Number(lone.stdout.trim()) : Number.NaN
    if (count !== 0) {
      const what = Number.isNaN(count) ? 'commits' : `${count} ${count === 1 ? 'commit' : 'commits'}`
      lost.push(`${what} on a detached HEAD that no branch holds`)
    }
  }
  // Git refuses a worktree with a populated gitlink (a submodule, mapped in
  // .gitmodules or not), or with submodule repositories kept in its own git
  // dir (they outlive a deinit).
  const index = await git(host, wt.path, ['ls-files', '--stage', '-z'])
  let populated = false
  for (const entry of index.exit_code === 0 ? index.stdout.split('\0') : []) {
    if (!entry.startsWith('160000 ')) continue
    const dir = joinPath(wt.path, entry.slice(entry.indexOf('\t') + 1))
    const top = await git(host, dir, ['rev-parse', '--show-toplevel']).catch(() => null)
    if (top?.exit_code === 0 && top.stdout.trim() === dir) {
      populated = true
      break
    }
  }
  const modules = await git(host, wt.path, ['rev-parse', '--path-format=absolute', '--git-path', 'modules'])
  const kept =
    modules.exit_code === 0 &&
    (await workspaceValidate(host, modules.stdout.trim()).then(
      () => true,
      () => false,
    ))
  if (populated || kept) lost.push('submodule repositories')
  if (lost.length > 0) {
    const listed = lost.length === 1 ? lost[0] : `${lost.slice(0, -1).join(', ')} and ${lost[lost.length - 1]}`
    throw Object.assign(new Error(`${name} has ${listed}`), { dirty: true })
  }
}

/** `path` is still a worktree git knows, with its folder in place and no
    tracked file gone: a removal killed part-way leaves it listed but not
    intact, and nothing should be moved back into that. */
export async function isIntactWorktree(host: Host, list: WorktreeList, path: string): Promise<boolean> {
  const main = list.worktrees[0]
  if (!main) return false
  const worktrees = await readWorktrees(host, main.path).catch(() => [])
  if (!worktrees.some((wt) => wt.path === path && !wt.prunable)) return false
  const status = await git(host, path, ['--no-optional-locks', 'status', '--porcelain', '--untracked-files=no']).catch(
    () => null,
  )
  return status?.exit_code === 0 && !/^(?:.D|D.) /m.test(status.stdout)
}

/** `wt remove`: delete the worktree, then its branch, but only when the
    default branch already has that branch, merged or squash-merged. It
    refuses first whatever `checkRemovable` refuses. An entry whose folder
    is already gone just drops out of git's list. */
export async function removeWorktree(
  host: Host,
  list: WorktreeList,
  wt: Worktree,
  force = false,
): Promise<{ branchDeleted: boolean; branchError?: string }> {
  await checkRemovable(host, list, wt, force)
  const main = list.worktrees[0]
  await run(
    host,
    main.path,
    // The `-c` makes git's own clean check count untracked files as well.
    ['-c', 'status.showUntrackedFiles=normal', 'worktree', 'remove', ...(force ? ['--force'] : []), wt.path],
    'git worktree remove',
    TREE_TIMEOUT_MS,
  )
  const target = list.defaultBranch
  if (wt.branch === null || target === null || wt.branch === target) return { branchDeleted: false }
  if (!(await integrated(host, main.path, wt.branch, target))) return { branchDeleted: false }
  // The worktree is gone by now: a branch that will not go (a stale ref
  // lock) is reported with it, not as a failed removal.
  try {
    await run(host, main.path, ['branch', '-D', wt.branch], 'git branch -D')
    return { branchDeleted: true }
  } catch (err) {
    return { branchDeleted: false, branchError: errorMessage(err) }
  }
}

/** `target` already has `branch`: either `branch` is its ancestor, or
    merging `branch` in would leave `target`'s tree unchanged. The second
    case is how a squash-merged pull request shows up. Any git failure
    keeps the branch. */
async function integrated(host: Host, cwd: string, branch: string, target: string): Promise<boolean> {
  const ours = `refs/heads/${target}`
  const theirs = `refs/heads/${branch}`
  const ancestor = await git(host, cwd, ['merge-base', '--is-ancestor', theirs, ours])
  if (ancestor.exit_code === 0) return true
  // `merge.default` would name a driver for paths without the attribute.
  const merged = await git(host, cwd, ['-c', 'merge.default=text', 'merge-tree', '--write-tree', ours, theirs])
  if (merged.exit_code !== 0) return false
  const tree = await git(host, cwd, ['rev-parse', `${ours}^{tree}`])
  if (tree.exit_code !== 0 || merged.stdout.split('\n')[0] !== tree.stdout.trim()) return false
  // That only counts where git's own merge decided: a driver such as
  // `merge=ours` drops the branch's side and would pass for merged.
  const base = await git(host, cwd, ['merge-base', ours, theirs])
  if (base.exit_code !== 0) return false
  const changed = await git(host, cwd, ['diff', '--name-only', '-z', '--no-renames', base.stdout.trim(), theirs])
  if (changed.exit_code !== 0 || changed.stdout_truncated) return false
  const paths = changed.stdout.split('\0').filter(Boolean)
  const attrs = await git(host, cwd, ['check-attr', '-z', '--stdin', 'merge'], undefined, { stdin: paths.join('\0') })
  // `<path> NUL merge NUL <value> NUL` for each path, or it was not all read.
  const fields = attrs.stdout.split('\0')
  if (attrs.exit_code !== 0 || fields.length !== paths.length * 3 + 1) return false
  return fields.every((value, i) => i % 3 !== 2 || value === 'unspecified' || value === 'set' || value === 'text')
}

/** The subjects a squash would fold together, oldest first. This is the
    starting point for the squash message. */
export async function mergeMessage(host: Host, list: WorktreeList, wt: Worktree): Promise<string> {
  if (list.defaultBranch === null) return ''
  const out = await git(host, wt.path, ['log', '--reverse', '--format=%s', `refs/heads/${list.defaultBranch}..HEAD`])
  return out.exit_code === 0 ? out.stdout.trim() : ''
}

/** `wt merge` without hooks. The steps:
    1. Squash everything, uncommitted changes included, into one commit.
       Without squash, the tree must be clean instead.
    2. Rebase onto the default branch.
    3. Fast-forward the default branch: in the worktree that has it checked
       out now, or by ref when none does.
    The worktree itself stays; `removeWorktree` drops it. A rebase conflict
    is aborted, so the branch keeps its squashed but un-rebased state. */
export async function mergeWorktree(
  host: Host,
  list: WorktreeList,
  wt: Worktree,
  options: { squash: boolean; message: string },
): Promise<{ target: string; sha: string }> {
  const target = list.defaultBranch
  const branch = wt.branch
  if (target === null) throw new Error('no default branch to merge into')
  if (wt.main || branch === null || branch === target) {
    throw new Error(`${branch ?? 'a detached worktree'} cannot be merged into ${target}`)
  }
  const cwd = wt.path
  const onto = `refs/heads/${target}`
  const head = await git(host, cwd, ['symbolic-ref', '--quiet', 'HEAD'])
  if (head.stdout.trim() !== `refs/heads/${branch}`) {
    throw new Error(`${branch} is no longer checked out there (a rebase in progress?)`)
  }
  for (const ref of ['MERGE_HEAD', 'CHERRY_PICK_HEAD', 'REVERT_HEAD']) {
    const pending = await git(host, cwd, ['rev-parse', '--quiet', '--verify', ref])
    if (pending.exit_code === 0) throw new Error(`finish the ${ref.split('_')[0].toLowerCase()} in progress first`)
  }
  const status = await run(host, cwd, STATUS, 'git status')
  if (/^(?:U.|.U|AA|DD) /m.test(status.stdout)) throw new Error('resolve the conflicted files first')
  if (options.squash) {
    const message = options.message.trim()
    if (message === '') throw new Error('a commit message is required')
    await run(host, cwd, ['add', '-A'], 'git add', TREE_TIMEOUT_MS)
    const base = (await run(host, cwd, ['merge-base', onto, 'HEAD'], 'git merge-base')).stdout.trim()
    const changed = await git(host, cwd, ['diff', '--cached', '--quiet', base])
    if (changed.exit_code === 0) throw new Error(`nothing to merge: ${branch} adds nothing to ${target}`)
    // The commit can still fail (a hook, signing, the timeout). The branch
    // then goes back to its tip instead of staying rewound to `base`.
    const tip = (await run(host, cwd, ['rev-parse', 'HEAD'], 'git rev-parse')).stdout.trim()
    await run(host, cwd, ['reset', '--soft', base], 'git reset')
    try {
      await run(host, cwd, ['commit', '-q', '-m', message], 'git commit', TREE_TIMEOUT_MS)
    } catch (err) {
      const back = await git(host, cwd, ['reset', '--soft', tip])
      if (back.exit_code === 0) throw err
      throw new Error(`${errorMessage(err)}; ${branch} was not put back: git reset --soft ${tip} restores it`)
    }
  } else {
    if (status.stdout.trim() !== '') throw new Error('commit the changes first, or merge with squash')
    const ahead = await run(host, cwd, ['rev-list', '--count', `${onto}..HEAD`], 'git rev-list')
    if (ahead.stdout.trim() === '0') throw new Error(`nothing to merge: ${branch} adds nothing to ${target}`)
  }
  const rebase = await git(host, cwd, ['rebase', onto], TREE_TIMEOUT_MS)
  if (rebase.exit_code !== 0) {
    const conflicts = await git(host, cwd, ['diff', '--name-only', '--diff-filter=U'])
    const abort = await git(host, cwd, ['rebase', '--abort'], TREE_TIMEOUT_MS)
    const files = conflicts.stdout.split('\n').filter(Boolean)
    const reason = files.length > 0 ? `on conflicts in ${files.join(', ')}` : `: ${rebase.stderr.trim()}`
    // A git killed on the timeout leaves its index.lock, which then fails
    // the abort too: say what may be left rather than claim a clean stop.
    if (rebase.timed_out || abort.exit_code !== 0) {
      const failed = abort.exit_code === 0 ? '' : ` (git rebase --abort: ${abort.stderr.trim().split('\n')[0]})`
      throw new Error(
        `rebase onto ${target} stopped ${reason}${failed}; a rebase in progress or an index.lock may be left in ${cwd}`,
      )
    }
    throw new Error(`rebase onto ${target} stopped ${reason}; rebase in a terminal, then merge again`)
  }
  const sha = (await run(host, cwd, ['rev-parse', 'HEAD'], 'git rev-parse')).stdout.trim()
  await fastForward(host, cwd, target, sha)
  return { target, sha }
}

/** Moves `target` forward to `sha`: in the worktree that has it checked out
    now (not when a list was read), or by ref when none does. */
async function fastForward(host: Host, cwd: string, target: string, sha: string): Promise<void> {
  const onto = `refs/heads/${target}`
  const holder = (await readWorktrees(host, cwd)).find((other) => other.branch === target && !other.prunable)
  if (holder) {
    const now = await git(host, holder.path, ['symbolic-ref', '--quiet', 'HEAD'])
    if (now.stdout.trim() !== onto) {
      throw new Error(`${target} is no longer checked out in ${basename(holder.path)}; merge again`)
    }
    await run(host, holder.path, ['merge', '--ff-only', '-q', sha], 'git merge --ff-only', TREE_TIMEOUT_MS)
  } else {
    // Checked out nowhere: fetch moves the ref, only as a fast-forward, and
    // refuses a branch that is checked out, or being rebased, anywhere.
    await run(host, cwd, ['fetch', '--no-recurse-submodules', '.', `${sha}:${onto}`], 'git fetch')
  }
}

/** A branch no worktree has checked out: its tip, and how many of its
    commits the default branch lacks; 0 once the default branch has them,
    squash-merged included, and null when there is no default branch. */
export async function branchStanding(
  host: Host,
  list: WorktreeList,
  branch: string,
): Promise<{ tip: string; unmerged: number | null }> {
  const cwd = list.worktrees[0].path
  const tip = (
    await run(host, cwd, ['rev-parse', '--verify', `refs/heads/${branch}^{commit}`], 'git rev-parse')
  ).stdout.trim()
  const target = list.defaultBranch
  if (target === null) return { tip, unmerged: null }
  if (branch === target || (await integrated(host, cwd, branch, target))) return { tip, unmerged: 0 }
  const ahead = await run(host, cwd, ['rev-list', '--count', `refs/heads/${target}..${tip}`], 'git rev-list')
  return { tip, unmerged: Math.max(1, Number(ahead.stdout.trim()) || 1) }
}

/** `git branch -D` on a branch no worktree has, provided it still points at
    `tip`, where it was when the user was asked. Git itself refuses a branch
    that a worktree has checked out. */
export async function deleteBranch(host: Host, list: WorktreeList, branch: string, tip: string): Promise<void> {
  const cwd = list.worktrees[0].path
  if (branch === list.defaultBranch) throw new Error(`${branch} is the default branch`)
  const now = await git(host, cwd, ['rev-parse', '--verify', '--quiet', `refs/heads/${branch}^{commit}`])
  if (now.exit_code !== 0) throw new Error(`${branch} is gone already`)
  if (now.stdout.trim() !== tip) throw new Error(`${branch} moved since; delete it again`)
  // ponytail: a commit landing between that check and the delete goes with
  // it. `update-ref -d <ref> <tip>` would close the gap, but leaves the
  // branch's config behind.
  await run(host, cwd, ['branch', '-D', branch], 'git branch -D')
}

/** The subjects of `branch`'s commits the default branch lacks, oldest
    first: the starting point for its squash message. */
export async function branchMergeMessage(host: Host, list: WorktreeList, branch: string): Promise<string> {
  if (list.defaultBranch === null) return ''
  const range = `refs/heads/${list.defaultBranch}..refs/heads/${branch}`
  const out = await git(host, list.worktrees[0].path, ['log', '--reverse', '--format=%s', range])
  return out.exit_code === 0 ? out.stdout.trim() : ''
}

/** `wt merge` for a branch no worktree has, without checking it out: its
    changes squashed into one commit, or its commits replayed, on top of the
    default branch, which then fast-forwards as mergeWorktree's does. The
    branch itself is left for the caller to delete; `tip` is where it was. */
export async function mergeBranch(
  host: Host,
  list: WorktreeList,
  branch: string,
  options: { squash: boolean; message: string },
): Promise<{ target: string; sha: string; tip: string }> {
  const target = list.defaultBranch
  if (target === null) throw new Error('no default branch to merge into')
  if (branch === target) throw new Error(`${branch} cannot be merged into itself`)
  const cwd = list.worktrees[0].path
  const onto = `refs/heads/${target}`
  if ((await readWorktrees(host, cwd)).some((wt) => wt.branch === branch)) {
    throw new Error(`${branch} is checked out in a worktree now; merge it from that row`)
  }
  const tip = (
    await run(host, cwd, ['rev-parse', '--verify', `refs/heads/${branch}^{commit}`], 'git rev-parse')
  ).stdout.trim()
  const ahead = await run(host, cwd, ['rev-list', '--count', `${onto}..${tip}`], 'git rev-list')
  if (ahead.stdout.trim() === '0') throw new Error(`nothing to merge: ${target} has all of ${branch}`)
  let sha: string
  if (options.squash) {
    const message = options.message.trim()
    if (message === '') throw new Error('a commit message is required')
    const merged = await git(
      host,
      cwd,
      ['merge-tree', '--write-tree', '--name-only', '--no-messages', onto, tip],
      TREE_TIMEOUT_MS,
    )
    const [tree, ...conflicts] = merged.stdout.split('\n').filter(Boolean)
    if (merged.exit_code === 1) {
      throw new Error(`${branch} conflicts with ${target} in ${conflicts.join(', ')}; merge it in a worktree`)
    }
    if (merged.exit_code === 129) throw new Error('merging a branch without a worktree needs git 2.38 or newer')
    if (merged.exit_code !== 0 || tree === undefined) throw new Error(merged.stderr.trim() || 'git merge-tree failed')
    const now = await run(host, cwd, ['rev-parse', `${onto}^{tree}`], 'git rev-parse')
    if (now.stdout.trim() === tree) throw new Error(`nothing to merge: ${target} has ${branch}'s changes already`)
    sha = (await run(host, cwd, ['commit-tree', tree, '-p', onto, '-m', message], 'git commit-tree')).stdout.trim()
  } else {
    const contained = await git(host, cwd, ['merge-base', '--is-ancestor', onto, tip])
    if (contained.exit_code === 0) {
      sha = tip
    } else {
      // Printed, not applied: `target` may be checked out, and only the
      // fast-forward below moves it along with its files. An older replay
      // prints without being asked and rejects the option.
      // It names the new tip for the ref the range ends on, so the range
      // ends on the branch, and the old tip it reports must still be `tip`.
      const range = `${onto}..refs/heads/${branch}`
      let replay = await git(host, cwd, ['replay', '--ref-action=print', '--onto', onto, range], TREE_TIMEOUT_MS)
      if (replay.exit_code === 129) replay = await git(host, cwd, ['replay', '--onto', onto, range], TREE_TIMEOUT_MS)
      if (/not a git command/.test(replay.stderr)) {
        throw new Error('keeping the commits needs git 2.44 or newer (git replay); merge with squash')
      }
      if (replay.exit_code !== 0) {
        throw new Error(`${branch}'s commits conflict with ${target}; merge with squash, or in a worktree`)
      }
      const moved = replay.stdout.split('\n').find((line) => line.startsWith(`update refs/heads/${branch} `))
      if (moved === undefined) throw new Error('git replay gave no commit')
      const [, , next, was] = moved.split(' ')
      if (was !== tip) throw new Error(`${branch} moved while it was being merged; merge again`)
      sha = next
    }
  }
  await fastForward(host, cwd, target, sha)
  return { target, sha, tip }
}

/** Takes `name` only when it stands for itself as a branch name. */
async function plainBranchName(host: Host, cwd: string, name: string): Promise<string> {
  const clean = name.trim()
  // It expands `@{-1}` and the like into another branch's name.
  const valid = await git(host, cwd, ['check-ref-format', '--branch', clean])
  if (valid.exit_code !== 0 || valid.stdout.trim() !== clean) throw new Error(`${clean} is not a plain branch name`)
  return clean
}

/** `git branch <name> <start>`: a branch checked out nowhere, since a
    worktree opens it (worktrunk never checks out in place). Started from a
    remote branch, it tracks that branch. */
export async function createBranch(host: Host, list: WorktreeList, name: string, start: string): Promise<string> {
  const cwd = list.worktrees[0]?.path
  if (!cwd) throw new Error('not a git repository')
  const branch = await plainBranchName(host, cwd, name)
  await run(host, cwd, ['branch', branch, start], 'git branch')
  return branch
}

/** `git branch -m`: the branch takes its new name, and its upstream with
    it. A worktree that has it checked out keeps its folder. */
export async function renameBranch(host: Host, list: WorktreeList, from: string, to: string): Promise<string> {
  const cwd = list.worktrees[0]?.path
  if (!cwd) throw new Error('not a git repository')
  if (from === list.defaultBranch) throw new Error(`${from} is the default branch; it keeps its name`)
  const branch = await plainBranchName(host, cwd, to)
  await run(host, cwd, ['branch', '-m', from, branch], 'git branch -m')
  return branch
}

export interface Upstream {
  remote: string
  /** The branch's ref on the remote, `refs/heads/…`. */
  ref: string
  /** Its remote-tracking name, `origin/main`. */
  name: string
}

async function upstreamOf(host: Host, cwd: string, branch: string): Promise<Upstream | null> {
  const out = await run(
    host,
    cwd,
    [
      'for-each-ref',
      '--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream:short)',
      `refs/heads/${branch}`,
    ],
    'git for-each-ref',
  )
  const [remote = '', ref = '', name = ''] = out.stdout.trim().split('\0')
  return remote !== '' && ref !== '' ? { remote, ref, name } : null
}

/** Asks for no credentials: a remote that needs them fails at once
    instead of waiting on a prompt nobody sees. */
async function network(host: Host, cwd: string, args: string[], operation: string): Promise<void> {
  const out = await git(host, cwd, args, TREE_TIMEOUT_MS, { env: { GIT_TERMINAL_PROMPT: '0' } })
  if (out.timed_out) throw new Error(`${operation} timed out`)
  if (out.exit_code !== 0) {
    const lines = out.stderr.trim().split('\n')
    throw new Error(
      lines.find((line) => /^(fatal|error|!|\s*!)/.test(line.trim())) ?? lines.at(-1) ?? `${operation} failed`,
    )
  }
}

/** Update, as WebStorm's: fetches the branch's upstream, then
    fast-forwards the branch to it, in the worktree holding it if one does.
    A branch that diverged from its upstream is left for a merge or a
    rebase in its worktree. Returns what happened, in a few words. */
export async function updateBranch(host: Host, list: WorktreeList, branch: string): Promise<string> {
  const cwd = list.worktrees[0]?.path
  if (!cwd) throw new Error('not a git repository')
  const up = await upstreamOf(host, cwd, branch)
  if (up === null) throw new Error(`${branch} tracks no remote branch`)
  await network(host, cwd, ['fetch', '--no-recurse-submodules', '--quiet', up.remote, up.ref], 'git fetch')
  const read = async (spec: string) =>
    (await run(host, cwd, ['rev-parse', '--verify', `${spec}^{commit}`], 'git rev-parse')).stdout.trim()
  const [tip, upstream] = await Promise.all([read(`refs/heads/${branch}`), read(`refs/remotes/${up.name}`)])
  if (tip === upstream) return `${branch} is up to date with ${up.name}`
  const behindOnly = await git(host, cwd, ['merge-base', '--is-ancestor', tip, upstream])
  if (behindOnly.exit_code === 0) {
    await fastForward(host, cwd, branch, upstream)
    return `updated ${branch} to ${up.name} (${upstream.slice(0, 9)})`
  }
  const aheadOnly = await git(host, cwd, ['merge-base', '--is-ancestor', upstream, tip])
  if (aheadOnly.exit_code === 0) return `${branch} has everything ${up.name} has; push to share its commits`
  throw new Error(`${branch} and ${up.name} diverged: merge or rebase it in its worktree`)
}

export interface PushTarget extends Upstream {
  /** The branch has no upstream yet: the push sets this one. */
  setUpstream: boolean
  /** Commits the remote lacks, when its side is known. */
  ahead: number | null
}

/** Where `branch` pushes: its upstream, else the same name on `origin`
    (or the only remote), which then becomes its upstream. */
export async function pushTarget(host: Host, list: WorktreeList, branch: string): Promise<PushTarget> {
  const cwd = list.worktrees[0]?.path
  if (!cwd) throw new Error('not a git repository')
  const up = await upstreamOf(host, cwd, branch)
  if (up !== null) {
    const count = await git(host, cwd, ['rev-list', '--count', `refs/remotes/${up.name}..refs/heads/${branch}`])
    return { ...up, setUpstream: false, ahead: count.exit_code === 0 ? Number(count.stdout.trim()) : null }
  }
  const remotes = (await run(host, cwd, ['remote'], 'git remote')).stdout.split('\n').filter((line) => line !== '')
  const remote = remotes.includes('origin') ? 'origin' : remotes[0]
  if (remote === undefined) throw new Error('this repository has no remote to push to')
  return { remote, ref: `refs/heads/${branch}`, name: `${remote}/${branch}`, setUpstream: true, ahead: null }
}

/** `git push` of `branch` to `target`, never forced: a remote that moved
    on rejects it, and says so. */
export async function pushBranch(host: Host, list: WorktreeList, branch: string, target: PushTarget): Promise<void> {
  const cwd = list.worktrees[0]?.path
  if (!cwd) throw new Error('not a git repository')
  await network(
    host,
    cwd,
    ['push', ...(target.setUpstream ? ['--set-upstream'] : []), target.remote, `refs/heads/${branch}:${target.ref}`],
    'git push',
  )
}

/** `git worktree prune`: drops git's entries for worktrees whose folder is
    gone. Returns how many went. */
export async function pruneWorktrees(host: Host, list: WorktreeList): Promise<number> {
  const main = list.worktrees[0]
  if (!main) throw new Error('not a git repository')
  const out = await run(host, main.path, ['worktree', 'prune', '-v'], 'git worktree prune')
  return (out.stdout + out.stderr).split('\n').filter((line) => line.startsWith('Removing ')).length
}

/** `git fetch --all --prune`, which asks for no credentials: a remote that
    needs them fails instead of waiting for a prompt nobody sees. */
export async function fetchAll(host: Host, list: WorktreeList): Promise<void> {
  const main = list.worktrees[0]
  if (!main) throw new Error('not a git repository')
  const out = await git(host, main.path, ['fetch', '--all', '--prune', '--quiet'], TREE_TIMEOUT_MS, {
    env: { GIT_TERMINAL_PROMPT: '0' },
  })
  if (out.timed_out) throw new Error('git fetch timed out')
  if (out.exit_code !== 0) throw new Error(out.stderr.trim().split('\n').slice(-1)[0] || 'git fetch failed')
}
