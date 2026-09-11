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

async function git(host: Host, cwd: string, args: string[]): Promise<ExecResponse> {
  return host.iii.trigger<ExecResponse>('shell::exec', {
    command: 'git',
    args,
    cwd,
    timeout_ms: 30_000,
  })
}

function failure(out: ExecResponse, operation: string): string | null {
  if (out.timed_out) return `${operation} timed out`
  if (out.exit_code === null) return `${operation} terminated without an exit code`
  if (out.exit_code !== 0) return out.stderr.trim() || `${operation} exited ${out.exit_code}`
  return null
}

async function run(host: Host, root: string, args: string[], operation: string): Promise<ExecResponse> {
  const out = await git(host, root, args)
  const message = failure(out, operation)
  if (message !== null) throw new Error(message)
  return out
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
    change is undone on its own so one failure names one file. */
export async function gitDiscard(
  host: Host,
  root: string,
  changes: readonly Pick<GitChange, 'path' | 'status' | 'staged' | 'from'>[],
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
          const [result] = await coderDelete(host, [joinPath(root, step.path)], false)
          if (result && !result.success) throw new Error(result.error?.message ?? 'delete failed')
          break
        }
        case 'restore-rename': {
          await run(
            host,
            root,
            ['restore', '--source=HEAD', '--staged', '--worktree', '--', step.from],
            'git restore',
          )
          await gitUnstage(host, root, [step.path])
          const [result] = await coderDelete(host, [joinPath(root, step.path)], false)
          if (result && !result.success) throw new Error(result.error?.message ?? 'delete failed')
          break
        }
        case 'restore':
          await run(
            host,
            root,
            ['restore', '--source=HEAD', '--staged', '--worktree', '--', step.path],
            'git restore',
          )
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
export async function gitFileAtRef(
  host: Host,
  root: string,
  ref: string,
  path: string,
): Promise<string | null> {
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
