/* Source-control verbs the explorer offers on top of `git.ts`'s read-only
   plumbing: stage, unstage, discard, commit, tags, and a file-at-ref read
   for the compare view. Everything goes through `shell::exec` in argv
   form, cwd-scoped to the browsed root, so nothing is shell-tokenized. */

import type { Host } from '@iii-dev/console-ui'
import { coderDelete, joinPath } from './coder'
import type { GitChange, GitFileStatus } from './git'

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

/** Discard working-tree (and index) changes for the given files. Each
    change is undone on its own so one failure names one file.
    `keepAdded` only un-adds an added file, leaving it on disk as unversioned
    (the Rollback dialog's unticked "Delete local copies of added files"). */
export async function gitDiscard(
  host: Host,
  root: string,
  changes: readonly Pick<GitChange, 'path' | 'status' | 'staged' | 'from'>[],
  options: { keepAdded?: boolean } = {},
): Promise<{ path: string; error: string | null }[]> {
  const results: { path: string; error: string | null }[] = []
  for (const change of changes) {
    const step = discardStep(change)
    try {
      switch (step.kind) {
        case 'delete': {
          const [result] = await coderDelete(host, [joinPath(root, step.path)], false)
          if (result && !result.success) throw new Error(result.error?.message ?? 'delete failed')
          break
        }
        case 'unstage-delete': {
          await gitUnstage(host, root, [step.path])
          if (options.keepAdded) break
          const [result] = await coderDelete(host, [joinPath(root, step.path)], false)
          if (result && !result.success) throw new Error(result.error?.message ?? 'delete failed')
          break
        }
        case 'restore-rename': {
          await run(host, root, ['restore', '--source=HEAD', '--staged', '--worktree', '--', step.from], 'git restore')
          await gitUnstage(host, root, [step.path])
          const [result] = await coderDelete(host, [joinPath(root, step.path)], false)
          if (result && !result.success) throw new Error(result.error?.message ?? 'delete failed')
          break
        }
        case 'restore':
          await run(host, root, ['restore', '--source=HEAD', '--staged', '--worktree', '--', step.path], 'git restore')
          break
      }
      results.push({ path: change.path, error: null })
    } catch (error) {
      results.push({
        path: change.path,
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
  return results
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

/** `git stash apply` or, with `pop`, apply and drop on success. */
export async function gitStashApply(host: Host, root: string, ref: string, pop: boolean): Promise<void> {
  await run(host, root, ['stash', pop ? 'pop' : 'apply', ref], `git stash ${pop ? 'pop' : 'apply'}`)
}

export async function gitStashDrop(host: Host, root: string, ref: string): Promise<void> {
  await run(host, root, ['stash', 'drop', ref], 'git stash drop')
}

/** Check out a new branch at the stash's base and apply it there; drops the stash on success. */
export async function gitStashBranch(host: Host, root: string, name: string, ref: string): Promise<void> {
  await run(host, root, ['stash', 'branch', name.trim(), ref], 'git stash branch')
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
