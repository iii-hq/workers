/* Source-control verbs the explorer offers on top of `git.ts`'s read-only
   plumbing: stage, unstage, discard, commit, stash, patches, .gitignore,
   tags, and a file-at-ref read for the compare view. Everything goes through `shell::exec` in argv
   form, cwd-scoped to the browsed root, so nothing is shell-tokenized. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { coderDelete, coderReadFile, coderWriteFile, joinPath } from './coder'
import type { GitChange, GitFileStatus } from './git'
import { isMissingFileError } from './load-error'
import { basename } from './paths'

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
  stdout_truncated: boolean
  stderr_truncated: boolean
}

/** `git` in `cwd`. `stdin` is fed to it; `env` adds to its environment. */
export async function git(
  host: Host,
  cwd: string,
  args: string[],
  timeoutMs = 30_000,
  options: { stdin?: string; env?: Record<string, string> } = {},
): Promise<ExecResponse> {
  return host.iii.trigger<ExecResponse>(
    'shell::exec',
    {
      command: 'git',
      args,
      cwd,
      timeout_ms: timeoutMs,
      ...(options.stdin === undefined ? {} : { stdin: options.stdin }),
      ...(options.env === undefined ? {} : { env: options.env }),
    },
    // The bus must outwait the command: a push can take the whole cap.
    { timeoutMs: timeoutMs + 10_000 },
  )
}

/** Another git process (an agent's, another editor's) holding `index.lock`. */
export function isIndexLocked(stderr: string): boolean {
  return /index\.lock|could not write index/i.test(stderr)
}

function failure(out: ExecResponse, operation: string): string | null {
  if (out.timed_out) return `${operation} timed out`
  if (out.exit_code === null) return `${operation} terminated without an exit code`
  if (out.exit_code !== 0) {
    const detail = out.stderr.trim()
    if (isIndexLocked(detail)) return 'another git process kept this repository locked; try again in a moment'
    return detail || `${operation} exited ${out.exit_code}`
  }
  return null
}

/** Waits between attempts while another process holds `index.lock`: other
    editors, agents and terminals in the same repository run git too, and
    their locks last milliseconds. ~2.5 s in all before giving up. */
export const LOCK_RETRY_DELAYS_MS = [150, 300, 600, 1500]

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

/** Run one mutating git command. Git takes `index.lock` before it changes
    anything and gives up untouched when the lock is held, so a lock failure
    is retried; `safeToRetry` lets a caller veto that when a command may have
    got further (a stash that already stored its entry). */
export async function run(
  host: Host,
  root: string,
  args: string[],
  operation: string,
  timeoutMs?: number,
  safeToRetry: () => Promise<boolean> = async () => true,
): Promise<ExecResponse> {
  for (let attempt = 0; ; attempt++) {
    const out = await git(host, root, args, timeoutMs)
    const message = failure(out, operation)
    if (message === null) return out
    const locked = out.exit_code !== 0 && isIndexLocked(out.stderr)
    if (!locked || attempt >= LOCK_RETRY_DELAYS_MS.length || !(await safeToRetry())) throw new Error(message)
    await sleep(LOCK_RETRY_DELAYS_MS[attempt])
  }
}

/** `git add -A -- <paths>`: stages modifications, additions and
    deletions alike. Paths are root-relative; `--` keeps a name that looks
    like an option honest. */
export async function gitStage(host: Host, root: string, paths: readonly string[]): Promise<void> {
  if (paths.length === 0) return
  await run(host, root, ['add', '-A', '--', ...paths], 'git add')
}

export async function gitStageAll(host: Host, root: string): Promise<void> {
  await run(host, root, ['add', '-A', '--', '.'], 'git add')
}

/** Take paths out of the index. `git restore --staged` needs a HEAD; an
    unborn repository falls back to `git rm --cached`. */
export async function gitUnstage(host: Host, root: string, paths: readonly string[]): Promise<void> {
  if (paths.length === 0) return
  const out = await git(host, root, ['restore', '--staged', '--', ...paths])
  if (out.exit_code === 0) return
  const fallback = await git(host, root, ['rm', '-q', '--cached', '-r', '--', ...paths])
  const message = failure(fallback, 'git unstage')
  if (message !== null) throw new Error(out.stderr.trim() || message)
}

export async function gitUnstageAll(host: Host, root: string): Promise<void> {
  const out = await git(host, root, ['reset', '-q', '--', '.'])
  const message = failure(out, 'git reset')
  if (message !== null) throw new Error(message)
}

/** The plan for throwing away one change. Untracked files are simply
    removed; a rename restores its source and drops the new name; anything
    tracked goes back to HEAD in both index and worktree. */
export type DiscardStep =
  | { kind: 'delete'; path: string }
  | { kind: 'restore'; path: string }
  | { kind: 'restore-rename'; from: string; path: string }
  | { kind: 'unstage-delete'; path: string }

export function discardStep(change: Pick<GitChange, 'path' | 'status' | 'staged' | 'from'>): DiscardStep {
  if (change.status === 'untracked') return { kind: 'delete', path: change.path }
  if (change.status === 'added') return { kind: 'unstage-delete', path: change.path }
  if (change.status === 'renamed' && change.from !== undefined) {
    return { kind: 'restore-rename', from: change.from, path: change.path }
  }
  return { kind: 'restore', path: change.path }
}

/** Discard working-tree (and index) changes for the given files in a few
    calls for the lot, in each change's own order: every restore, then
    every unstage, then every delete. A call that fails is retried file by
    file, so a failure names its file, and a failed change skips the rest
    of its steps. `keepAdded` only un-adds an added file, leaving it on disk
    as unversioned (the Rollback dialog's unticked "Delete local copies of
    added files"). */
export async function gitDiscard(
  host: Host,
  root: string,
  changes: readonly Pick<GitChange, 'path' | 'status' | 'staged' | 'from'>[],
  options: { keepAdded?: boolean } = {},
): Promise<{ path: string; error: string | null }[]> {
  const steps = changes.map(discardStep)
  const failed = new Map<string, string>()
  await discardBatch(
    steps.flatMap((step) =>
      step.kind === 'restore'
        ? [{ change: step.path, path: step.path }]
        : step.kind === 'restore-rename'
          ? [{ change: step.path, path: step.from }]
          : [],
    ),
    async (paths) => {
      await run(host, root, ['restore', '--source=HEAD', '--staged', '--worktree', '--', ...paths], 'git restore')
      return []
    },
    failed,
  )
  await discardBatch(
    steps
      .filter((step) => step.kind === 'unstage-delete' || step.kind === 'restore-rename')
      .map((step) => ({ change: step.path, path: step.path })),
    async (paths) => {
      await gitUnstage(host, root, paths)
      return []
    },
    failed,
  )
  await discardBatch(
    steps
      .filter((step) => step.kind !== 'restore' && !(options.keepAdded && step.kind === 'unstage-delete'))
      .map((step) => ({ change: step.path, path: step.path })),
    async (paths) => {
      const results = await coderDelete(host, paths.map((path) => joinPath(root, path)), false)
      return paths.map((_, index) => {
        const result = results[index]
        return result && !result.success ? (result.error?.message ?? 'delete failed') : null
      })
    },
    failed,
  )
  return changes.map((change) => ({ path: change.path, error: failed.get(change.path) ?? null }))
}

/** One discard step over many paths, each tied to the change it undoes.
    `call` fails outright, or resolves to the failures it can name per path
    (by index; none listed means none failed). */
async function discardBatch(
  targets: readonly { change: string; path: string }[],
  call: (paths: string[]) => Promise<readonly (string | null)[]>,
  failed: Map<string, string>,
): Promise<void> {
  const live = targets.filter((target) => !failed.has(target.change))
  if (live.length === 0) return
  try {
    const errors = await call(live.map((target) => target.path))
    live.forEach((target, index) => {
      const error = errors[index]
      if (error) failed.set(target.change, error)
    })
  } catch (error) {
    if (live.length === 1) {
      failed.set(live[0].change, errorMessage(error))
      return
    }
    for (const target of live) await discardBatch([target], call, failed)
  }
}

/** Commit what is staged. */
export async function gitCommit(host: Host, root: string, message: string): Promise<string> {
  const trimmed = message.trim()
  if (trimmed === '') throw new Error('a commit message is required')
  await run(host, root, ['commit', '-q', '-m', trimmed], 'git commit')
  const out = await run(host, root, ['rev-parse', '--short', 'HEAD'], 'git rev-parse')
  return out.stdout.trim()
}

/** One commit from the Commit panel: exactly the ticked paths, whatever the index holds. */
export interface CommitRequest {
  message: string
  /** Root-relative; a rename lists both its source and its destination. */
  paths: readonly string[]
  amend: boolean
  /** `--signoff`: a Signed-off-by trailer. */
  signOff: boolean
  /** `--no-verify`: skip the pre-commit and commit-msg hooks. */
  noVerify: boolean
  /** `--author`, when set: `Name <email>` or a pattern git resolves. */
  author: string
}

/** The git commands a commit runs, in order. Ticked unversioned files have
    to be added before git will commit them by path; `git commit -- <paths>`
    then records the working copy of just those paths (`--only` is the default
    with paths), so the other staged or unstaged changes stay put. An amend
    with nothing ticked rewrites only the message (`--only`, no paths). */
export function commitCommands(request: CommitRequest): string[][] {
  const message = request.message.trim()
  if (message === '') throw new Error('a commit message is required')
  if (request.paths.length === 0 && !request.amend) throw new Error('select the changes to commit')
  const flags = [
    ...(request.amend ? ['--amend'] : []),
    ...(request.signOff ? ['--signoff'] : []),
    ...(request.noVerify ? ['--no-verify'] : []),
    ...(request.author.trim() ? [`--author=${request.author.trim()}`] : []),
  ]
  if (request.paths.length === 0) return [['commit', '-q', ...flags, '--only', '-m', message]]
  return [
    ['add', '-A', '--', ...request.paths],
    ['commit', '-q', ...flags, '-m', message, '--', ...request.paths],
  ]
}

/** Run a commit request; returns the new short sha. */
export async function gitCommitChanges(host: Host, root: string, request: CommitRequest): Promise<string> {
  for (const args of commitCommands(request)) {
    // Hooks may run a formatter or the test suite: give them the long cap.
    await run(host, root, args, `git ${args[0]}`, 120_000)
  }
  const out = await run(host, root, ['rev-parse', '--short', 'HEAD'], 'git rev-parse')
  return out.stdout.trim()
}

/** Push the current branch; a branch with no upstream yet gets one on `origin`. */
export async function gitPush(host: Host, root: string): Promise<void> {
  const out = await git(host, root, ['push'], 120_000)
  if (out.exit_code === 0) return
  if (/has no upstream branch|no upstream configured/i.test(out.stderr)) {
    await run(host, root, ['push', '--set-upstream', 'origin', 'HEAD'], 'git push', 120_000)
    return
  }
  const message = failure(out, 'git push')
  if (message !== null) throw new Error(message)
}

/** A new commit that undoes `sha`. */
export async function gitRevert(host: Host, root: string, sha: string): Promise<void> {
  await run(host, root, ['revert', '--no-edit', sha], 'git revert', 120_000)
}

/** A branch at `at`, without switching to it. */
export async function gitCreateBranch(host: Host, root: string, name: string, at: string): Promise<void> {
  await run(host, root, ['branch', name.trim(), at], 'git branch')
}

export interface StashRequest {
  message: string
  includeUntracked: boolean
  /** Only these root-relative paths (a rename lists both ends); empty stashes the whole working tree. */
  paths?: readonly string[]
}

/** `git stash push`, limited to `paths` when given: the rest of the working tree stays as it is. */
export function stashPushArgs(request: StashRequest): string[] {
  const message = request.message.trim()
  const paths = request.paths ?? []
  return [
    'stash',
    'push',
    ...(request.includeUntracked ? ['--include-untracked'] : []),
    ...(message ? ['-m', message] : []),
    ...(paths.length > 0 ? ['--', ...paths] : []),
  ]
}

/** The newest stash's object id, or '' when there is none. */
async function stashTip(host: Host, root: string): Promise<string> {
  const out = await git(host, root, ['--no-optional-locks', 'rev-parse', '-q', '--verify', 'refs/stash'])
  return out.exit_code === 0 ? out.stdout.trim() : ''
}

export async function gitStashPush(host: Host, root: string, request: StashRequest): Promise<void> {
  // A lock failure is retried only while no new stash entry exists: one that
  // got as far as storing it must not be stashed a second time.
  const before = await stashTip(host, root)
  await run(
    host,
    root,
    stashPushArgs(request),
    'git stash',
    undefined,
    async () => (await stashTip(host, root)) === before,
  )
}

/** `git stash apply` or, with `pop`, apply and drop on success; `index`
    stages again what was staged when the stash was made. */
export async function gitStashApply(host: Host, root: string, ref: string, pop: boolean, index = false): Promise<void> {
  const verb = pop ? 'pop' : 'apply'
  await run(host, root, ['stash', verb, ...(index ? ['--index'] : []), ref], `git stash ${verb}`)
}

export async function gitStashDrop(host: Host, root: string, ref: string): Promise<void> {
  await run(host, root, ['stash', 'drop', ref], 'git stash drop')
}

/** Drops every stash. */
export async function gitStashClear(host: Host, root: string): Promise<void> {
  await run(host, root, ['stash', 'clear'], 'git stash clear')
}

const PATCH_FLAGS = ['--binary', '--no-color', '--no-ext-diff', '--no-textconv']
// ponytail: one `git diff --no-index` per unversioned file; past this many
// a patch is refused rather than run for minutes.
export const PATCH_UNVERSIONED_LIMIT = 200

/** The working tree's changes to `tracked` (against HEAD) and the whole of
    the `untracked` files, as one patch with root-relative paths: `git apply`
    in the root takes it back. Rename sources go in `tracked` too. */
export async function gitLocalPatch(
  host: Host,
  root: string,
  tracked: readonly string[],
  untracked: readonly string[],
): Promise<string> {
  if (untracked.length > PATCH_UNVERSIONED_LIMIT) {
    throw new Error(`${untracked.length} unversioned files are too many for one patch`)
  }
  const parts: string[] = []
  const take = (out: ExecResponse) => {
    if (out.stdout_truncated) throw new Error('the changes are too large to use here')
    parts.push(out.stdout)
  }
  if (tracked.length > 0) {
    const args = ['--no-optional-locks', 'diff', 'HEAD', '--relative', '-M', ...PATCH_FLAGS, '--', ...tracked]
    take(await run(host, root, args, 'git diff'))
  }
  for (const path of untracked) {
    // Exits 1 when the two differ, which a new file always does.
    const args = ['--no-optional-locks', 'diff', '--no-index', ...PATCH_FLAGS, '--', '/dev/null', path]
    const out = await git(host, root, args)
    if (out.exit_code !== 1) {
      const message = failure(out, 'git diff')
      if (message !== null) throw new Error(message)
    }
    take(out)
  }
  const patch = parts.join('')
  if (patch.trim() === '') throw new Error('there are no changes to put in a patch')
  return patch
}

/** A root-relative path as a line of the root's .gitignore: anchored with a
    leading `/`, git's pattern characters and trailing spaces escaped so it
    matches that one path. A folder keeps its trailing `/`. */
export function ignorePattern(path: string): string {
  return `/${path.replace(/[\\*?[\]]/g, '\\$&').replace(/ (?= *$)/g, '\\ ')}`
}

/** `existing` with a line for each path it does not list yet, and how many
    that is; null when it lists them all. */
export function withIgnored(existing: string, paths: readonly string[]): { content: string; added: number } | null {
  const lines = new Set(existing.split(/\r?\n/))
  const added = [...new Set(paths.map(ignorePattern))].filter((line) => !lines.has(line))
  if (added.length === 0) return null
  // A CRLF file stays CRLF rather than ending up with mixed line endings.
  const eol = existing.includes('\r\n') ? '\r\n' : '\n'
  const base = existing === '' || existing.endsWith('\n') ? existing : `${existing}${eol}`
  return { content: `${base}${added.join(eol)}${eol}`, added: added.length }
}

/** Lists root-relative `paths` in the root's .gitignore, made when missing.
    Resolves to how many lines were added. */
export async function gitIgnore(host: Host, root: string, paths: readonly string[]): Promise<number> {
  const file = joinPath(root, '.gitignore')
  let existing = ''
  let mode: number | null = null
  let revision: string | null = null
  try {
    const out = await coderReadFile(host, file, { maxOutputBytes: 8 * 1024 * 1024 })
    if (out.is_utf8 === false || out.more_lines) throw new Error('.gitignore is too large or not text')
    existing = out.content ?? ''
    mode = out.mode ?? null
    revision = out.revision ?? null
  } catch (err) {
    // The bus rejects with the handler's error body, not an Error.
    if (!isMissingFileError(errorMessage(err))) throw err
  }
  const next = withIgnored(existing, paths)
  if (next === null) return 0
  // Against the revision read: an edit made since fails the write rather than being lost.
  const result = await coderWriteFile(host, file, next.content, mode, revision)
  if (!result.success) throw new Error(result.error?.message ?? 'could not write .gitignore')
  return next.added
}

/** Check out a new branch at the stash's base and apply it there; drops the stash on success. */
export async function gitStashBranch(host: Host, root: string, name: string, ref: string): Promise<void> {
  await run(host, root, ['stash', 'branch', name.trim(), ref], 'git stash branch')
}

/** The repository's top level: `git apply` below it skips the paths
    outside the folder it runs in. */
async function topOf(host: Host, cwd: string): Promise<string> {
  return (await run(host, cwd, ['rev-parse', '--show-toplevel'], 'git rev-parse')).stdout.trim()
}

const inRepo = (paths: readonly string[]) => paths.map((path) => `:(top,literal)${path}`)

/** What commit `sha` changed in `paths` (repository-relative), as a patch
    against its first parent (everything for a root commit), binary files
    included and untouched by local diff settings. */
export async function gitCommitPatch(host: Host, cwd: string, sha: string, paths: readonly string[]): Promise<string> {
  const flags = ['--format=', '--binary', '--no-color', '--no-ext-diff', '--no-textconv', '--diff-merges=first-parent']
  const patch = await run(host, cwd, ['show', ...flags, sha, '--', ...inRepo(paths)], 'git show')
  if (patch.stdout_truncated) throw new Error('the changes are too large to use here')
  if (patch.stdout.trim() === '') throw new Error('the commit changed none of these files')
  return patch.stdout
}

/** Applies what commit `sha` changed in `paths` to the working tree only,
    or undoes it (`reverse`): a cherry-pick or a revert of those files,
    neither staged nor committed. The patch applies whole or not at all,
    so a file edited since refuses it rather than half-changing. */
export async function gitApplyCommitChanges(
  host: Host,
  cwd: string,
  sha: string,
  paths: readonly string[],
  reverse: boolean,
): Promise<void> {
  if (paths.length === 0) return
  const top = await topOf(host, cwd)
  const patch = await gitCommitPatch(host, top, sha, paths)
  const args = ['apply', ...(reverse ? ['-R'] : []), '--whitespace=nowarn']
  const out = await git(host, top, args, undefined, { stdin: patch })
  const message = failure(out, 'git apply')
  if (message !== null) throw new Error(message)
}

/** Puts `paths` in the working tree as they are at `sha`, a path absent
    there removed; the index stays. What was in those files is lost. */
export async function gitRestoreFrom(host: Host, cwd: string, sha: string, paths: readonly string[]): Promise<void> {
  if (paths.length === 0) return
  const top = await topOf(host, cwd)
  // A path neither the index nor the commit knows is either gone already
  // (the commit deleted it, and so does the working tree) or a file made
  // again since and never added, which is not git's to remove: git restore
  // refuses both with a bare pathspec error, so tell them apart.
  const known = await run(host, top, ['ls-files', '-z', `--with-tree=${sha}`, '--', ...inRepo(paths)], 'git ls-files')
  const listed = new Set(known.stdout.split('\0'))
  const unknown = paths.filter((path) => !listed.has(path))
  if (unknown.length > 0) {
    const others = await run(host, top, ['ls-files', '-z', '--others', '--', ...inRepo(unknown)], 'git ls-files')
    const onDisk = others.stdout.split('\0').find(Boolean)
    if (onDisk !== undefined) throw new Error(`${basename(onDisk)} is not tracked: left as is`)
  }
  const restorable = paths.filter((path) => listed.has(path))
  if (restorable.length === 0) return
  await run(host, top, ['restore', `--source=${sha}`, '--worktree', '--', ...inRepo(restorable)], 'git restore')
}

export interface GitTagSummary {
  name: string
  sha: string
}

/** Tags, newest first by creation date. */
export async function gitTags(host: Host, root: string): Promise<GitTagSummary[]> {
  const out = await run(
    host,
    root,
    ['for-each-ref', '--sort=-creatordate', '--format=%(refname:short)%00%(objectname)', 'refs/tags/'],
    'git for-each-ref',
  )
  if (out.stdout === '') return []
  const tags: GitTagSummary[] = []
  for (const line of out.stdout.split('\n')) {
    if (line === '') continue
    const [name, sha] = line.split('\0')
    if (!name || !sha) continue
    tags.push({ name, sha })
  }
  return tags
}

/** One side of a compare: the file as committed at `ref`. `null` when the
    path did not exist there (an addition relative to that ref). Throws on
    an unknown ref or binary content. */
export async function gitFileAtRef(host: Host, root: string, ref: string, path: string): Promise<string | null> {
  const out = await git(host, root, ['show', `${ref}:./${path}`])
  if (out.exit_code !== 0) {
    const detail = out.stderr.trim()
    if (/exists on disk, but not in|does not exist in/.test(detail)) return null
    if (/invalid object name|unknown revision|bad revision|not a valid object/i.test(detail)) {
      throw new Error(`unknown revision: ${ref}`)
    }
    throw new Error(detail || `git show exited ${out.exit_code}`)
  }
  if (out.stdout_truncated) throw new Error('file is larger than the shell output cap')
  if (out.stdout.includes('\0') || out.stdout.includes('�')) {
    throw new Error(`binary file: ${path}`)
  }
  return out.stdout
}

/** The letter VS Code puts beside a changed file. */
export function statusLetter(status: GitFileStatus): string {
  switch (status) {
    case 'added':
      return 'A'
    case 'deleted':
      return 'D'
    case 'modified':
      return 'M'
    case 'renamed':
      return 'R'
    case 'untracked':
      return 'U'
    case 'ignored':
      return 'I'
  }
}

export function statusTitle(status: GitFileStatus): string {
  switch (status) {
    case 'added':
      return 'Added'
    case 'deleted':
      return 'Deleted'
    case 'modified':
      return 'Modified'
    case 'renamed':
      return 'Renamed'
    case 'untracked':
      return 'Untracked'
    case 'ignored':
      return 'Ignored'
  }
}
