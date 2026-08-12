/* Git plumbing for the explorer's git tab — everything goes through the
   worker's own `shell::exec` (argv form, so nothing is shell-tokenized),
   scoped to the browsed root via `cwd`. */

import type { Host } from '@iii-dev/console-ui'

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
}

async function git(
  host: Host,
  cwd: string,
  args: string[],
): Promise<ExecResponse> {
  return host.iii.trigger<ExecResponse>('shell::exec', {
    command: 'git',
    args,
    cwd,
    timeout_ms: 15_000,
  })
}

/** Mirrors @pierre/trees' GitStatus vocabulary so entries feed
    `model.setGitStatus` directly. */
export type GitFileStatus =
  | 'added'
  | 'deleted'
  | 'modified'
  | 'renamed'
  | 'untracked'
  | 'ignored'

export interface GitChange {
  /** Root-relative path (git speaks toplevel-relative; the page browses
      the toplevel's subtree, see `gitChanges`). */
  path: string
  status: GitFileStatus
  /** Rename source, when status is 'renamed'. */
  from?: string
  /** True when the change is staged (X column), for the row hint. */
  staged: boolean
}

export type GitState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; changes: GitChange[] }

function statusFromCode(x: string, y: string): GitFileStatus | null {
  if (x === '?' || y === '?') return 'untracked'
  if (x === '!' || y === '!') return 'ignored'
  // Worktree column wins for display; fall back to the index column.
  const code = y !== ' ' && y !== '' ? y : x
  switch (code) {
    case 'A':
      return 'added'
    case 'D':
      return 'deleted'
    case 'R':
      return 'renamed'
    case 'M':
    case 'T':
    case 'U':
    case 'C':
      return 'modified'
    default:
      return null
  }
}

/** `git status --porcelain -z -- .` over the browsed root. `-z` gives
    NUL-separated records with verbatim paths (no quoting to undo);
    rename records carry the OLD path as a second NUL field. Status
    paths are repo-TOPLEVEL-relative, so when the browsed root is a
    subdirectory the `--show-prefix` is stripped to keep the page's
    root-relative vocabulary (the `-- .` pathspec already scopes the
    report to the subtree). */
export async function gitChanges(host: Host, root: string): Promise<GitState> {
  const probe = await git(host, root, ['rev-parse', '--is-inside-work-tree'])
  if (probe.exit_code !== 0 || !probe.stdout.startsWith('true')) {
    return { kind: 'not-a-repo' }
  }

  const prefixOut = await git(host, root, ['rev-parse', '--show-prefix'])
  const prefix = prefixOut.exit_code === 0 ? prefixOut.stdout.trim() : ''
  const toRootRel = (p: string) =>
    prefix !== '' && p.startsWith(prefix) ? p.slice(prefix.length) : p

  // `-uall` lists untracked files individually — without it a new folder
  // collapses to one `folder/` record, which is unclickable (a directory
  // has no diff) and hides what's actually new.
  const out = await git(host, root, [
    'status',
    '--porcelain',
    '-z',
    '-uall',
    '--',
    '.',
  ])
  if (out.exit_code !== 0) {
    return {
      kind: 'error',
      message: out.stderr.trim() || `git status exited ${out.exit_code}`,
    }
  }

  const fields = out.stdout.split('\0')
  const changes: GitChange[] = []
  for (let i = 0; i < fields.length; i++) {
    const record = fields[i]
    if (record.length < 4) continue
    const x = record[0]
    const y = record[1]
    const path = record.slice(3)
    const status = statusFromCode(x, y)
    if (!status) continue
    const change: GitChange = {
      path: toRootRel(path),
      status,
      staged: x !== ' ' && x !== '?' && x !== '!',
    }
    if (x === 'R' || y === 'R') {
      // The next NUL field is the rename source.
      change.from = toRootRel(fields[++i])
    }
    changes.push(change)
  }
  return { kind: 'ready', changes }
}

/** The file's content at HEAD, or '' when it didn't exist there
    (added/untracked). Paths are toplevel-relative on the git side —
    `:./<path>` anchors the pathspec to `cwd` instead. */
export async function gitShowHead(
  host: Host,
  root: string,
  path: string,
): Promise<string> {
  const out = await git(host, root, ['show', `HEAD:./${path}`])
  return out.exit_code === 0 ? out.stdout : ''
}
