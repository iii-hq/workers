/* Git plumbing for the explorer's git tab — everything goes through the
   worker's own `shell::exec` (argv form, so nothing is shell-tokenized),
   scoped to the browsed root via `cwd`. */

import type { Host } from '@iii-dev/console-ui'
import { coderReadFile, joinPath } from './coder'

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
  stdout_truncated: boolean
  stderr_truncated: boolean
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
  /** Rename source, when this comparison crosses a rename. */
  from?: string
  /** True when the change is staged (X column), for the row hint. */
  staged: boolean
}

export type GitState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; changes: GitChange[] }

/** The three useful snapshots exposed by the review UI. */
export type GitComparisonScope = 'uncommitted' | 'unstaged' | 'staged'

/** A content endpoint for a comparison. Paths are relative to `root`, so
    consumers can resolve worktree sources through `coder::read-file` and Git
    sources through `git show <revision>:./path` / `git show :./path`. */
export type GitContentSource =
  | { kind: 'empty' }
  | { kind: 'head'; path: string }
  | { kind: 'index'; path: string }
  | { kind: 'worktree'; path: string }
  | { kind: 'revision'; revision: string; path: string }

export interface GitResolvedComparisonEntry extends GitChange {
  /** The original path carried by a porcelain rename record. This remains
      available when a later worktree edit makes the display status modified. */
  renameFrom?: string
  before: GitContentSource
  after: GitContentSource
}

/** One worktree/index comparison. `x` and `y` are the porcelain columns
    even when only one side selects the entry. A staged deletion followed by
    an untracked recreation combines the X and Y state from two records. */
export interface GitComparisonEntry extends GitResolvedComparisonEntry {
  x: string
  y: string
}

export type GitComparisonState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; scope: GitComparisonScope; changes: GitComparisonEntry[] }

export interface GitCommitSummary {
  sha: string
  subject: string
}

export type GitRecentCommitsState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; commits: GitCommitSummary[] }

export interface GitRefSummary {
  kind: 'local' | 'remote'
  /** Human-facing branch name (`main` or `origin/main`). */
  name: string
  /** Unambiguous Git ref name (`refs/heads/main`). */
  fullName: string
  sha: string
  current: boolean
}

export type GitRefsState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; refs: GitRefSummary[] }

interface PorcelainEntry {
  path: string
  x: string
  y: string
  renameFrom?: string
}

type PorcelainState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; prefix: string; entries: PorcelainEntry[] }

type RepositoryState =
  | { kind: 'not-a-repo' }
  | { kind: 'error'; message: string }
  | { kind: 'ready' }

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function execFailure(out: ExecResponse, operation: string): string | null {
  if (out.timed_out) return `${operation} timed out`
  if (out.stdout_truncated || out.stderr_truncated) {
    const streams = [out.stdout_truncated ? 'stdout' : '', out.stderr_truncated ? 'stderr' : '']
      .filter(Boolean)
      .join(' and ')
    return `${operation} ${streams} ${streams.includes(' and ') ? 'were' : 'was'} truncated`
  }
  if (out.exit_code === null) return `${operation} terminated without an exit code`
  if (out.exit_code !== 0) return out.stderr.trim() || `${operation} exited ${out.exit_code}`
  return null
}

async function probeRepository(host: Host, root: string): Promise<RepositoryState> {
  try {
    const probe = await git(host, root, ['rev-parse', '--is-inside-work-tree'])
    const failure = execFailure(probe, 'git rev-parse')
    if (failure !== null) {
      // A normal non-zero rev-parse is the expected non-repository signal;
      // timeout, truncation, signal termination, and safety failures are not.
      if (
        !probe.timed_out &&
        !probe.stdout_truncated &&
        !probe.stderr_truncated &&
        probe.exit_code !== null &&
        probe.exit_code !== 0
      ) {
        const detail = probe.stderr.trim()
        if (detail === '' || detail.toLowerCase().includes('not a git repository')) {
          return { kind: 'not-a-repo' }
        }
      }
      return { kind: 'error', message: failure }
    }
    return probe.stdout.startsWith('true') ? { kind: 'ready' } : { kind: 'not-a-repo' }
  } catch (error) {
    return { kind: 'error', message: `git execution failed: ${errorMessage(error)}` }
  }
}

async function checkedGit(
  host: Host,
  root: string,
  args: string[],
  operation: string,
): Promise<ExecResponse> {
  const out = await git(host, root, args)
  const failure = execFailure(out, operation)
  if (failure !== null) throw new Error(failure)
  return out
}

function caughtGitMessage(error: unknown): string {
  const message = errorMessage(error)
  return message.startsWith('git ') ? message : `git execution failed: ${message}`
}

function rootRelative(prefix: string, path: string): string {
  return prefix !== '' && path.startsWith(prefix) ? path.slice(prefix.length) : path
}

interface NameStatusEntry {
  path: string
  status: Exclude<GitFileStatus, 'untracked' | 'ignored'>
  from?: string
}

function diffStatus(code: string): NameStatusEntry['status'] | null {
  switch (code[0]) {
    case 'A':
    case 'C':
      return 'added'
    case 'D':
      return 'deleted'
    case 'R':
      return 'renamed'
    case 'M':
    case 'T':
    case 'U':
    case 'X':
    case 'B':
      return 'modified'
    default:
      return null
  }
}

/** Parse `git diff --name-status -z`: status, then one path; rename/copy
    records carry old and new paths. NUL framing keeps all legal path bytes
    except NUL unambiguous. */
function parseNameStatus(stdout: string, prefix: string): NameStatusEntry[] | string {
  if (stdout === '') return []
  if (!stdout.endsWith('\0')) return 'git diff returned an incomplete name-status record'

  const fields = stdout.slice(0, -1).split('\0')
  const entries: NameStatusEntry[] = []
  for (let i = 0; i < fields.length; ) {
    const code = fields[i++]
    if (!/^[A-Z][0-9]*$/.test(code)) return 'git diff returned malformed name-status data'
    const status = diffStatus(code)
    if (status === null) return `git diff returned unsupported status ${code}`

    if (code[0] === 'R' || code[0] === 'C') {
      const from = fields[i++]
      const path = fields[i++]
      if (from === undefined || path === undefined || from === '' || path === '') {
        return 'git diff returned an incomplete rename record'
      }
      const entry: NameStatusEntry = { path: rootRelative(prefix, path), status }
      if (code[0] === 'R') entry.from = rootRelative(prefix, from)
      entries.push(entry)
      continue
    }

    const path = fields[i++]
    if (path === undefined || path === '') return 'git diff returned an incomplete path record'
    entries.push({ path: rootRelative(prefix, path), status })
  }
  return entries
}

function parsePorcelain(stdout: string, prefix: string): PorcelainEntry[] | string {
  if (stdout === '') return []
  if (!stdout.endsWith('\0')) return 'git status returned an incomplete porcelain record'

  const fields = stdout.slice(0, -1).split('\0')
  const entries: PorcelainEntry[] = []
  const toRootRel = (path: string) =>
    prefix !== '' && path.startsWith(prefix) ? path.slice(prefix.length) : path

  for (let i = 0; i < fields.length; i++) {
    const record = fields[i]
    if (record.length < 4 || record[2] !== ' ') {
      return 'git status returned malformed porcelain data'
    }
    const x = record[0]
    const y = record[1]
    const path = record.slice(3)
    if (path === '') return 'git status returned an empty path'

    const entry: PorcelainEntry = { path: toRootRel(path), x, y }
    // With `-z`, Git emits the destination in the status record and the
    // original pathname in the following NUL field (without `old -> new`).
    if (x === 'R' || y === 'R' || x === 'C' || y === 'C') {
      const from = fields[++i]
      if (from === undefined || from === '') {
        return 'git status returned an incomplete rename record'
      }
      if (x === 'R' || y === 'R') entry.renameFrom = toRootRel(from)
    }
    entries.push(entry)
  }
  return entries
}

function recreatedAfterStagedDelete(entries: PorcelainEntry[]): Set<string> {
  const deleted = new Set(
    entries
      .filter((entry) => entry.x === 'D' && entry.y === ' ')
      .map((entry) => entry.path),
  )
  return new Set(
    entries
      .filter((entry) => entry.x === '?' && entry.y === '?' && deleted.has(entry.path))
      .map((entry) => entry.path),
  )
}

async function porcelainStatus(host: Host, root: string): Promise<PorcelainState> {
  const repository = await probeRepository(host, root)
  if (repository.kind !== 'ready') return repository

  try {
    const prefixOut = await git(host, root, ['rev-parse', '--show-prefix'])
    const prefixFailure = execFailure(prefixOut, 'git rev-parse --show-prefix')
    if (prefixFailure !== null) return { kind: 'error', message: prefixFailure }
    // Strip rev-parse's line terminator without corrupting a legal leading
    // space in a directory name.
    const prefix = prefixOut.stdout.replace(/\r?\n$/, '')

    // `--untracked-files=all` lists new files individually; `--renames`
    // makes the rename contract explicit rather than depending on config.
    const out = await git(host, root, [
      'status',
      '--porcelain=v1',
      '-z',
      '--untracked-files=all',
      '--renames',
      '--',
      '.',
    ])
    const statusFailure = execFailure(out, 'git status')
    if (statusFailure !== null) return { kind: 'error', message: statusFailure }

    const entries = parsePorcelain(out.stdout, prefix)
    return typeof entries === 'string'
      ? { kind: 'error', message: entries }
      : { kind: 'ready', prefix, entries }
  } catch (error) {
    return { kind: 'error', message: `git execution failed: ${errorMessage(error)}` }
  }
}

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
  const state = await porcelainStatus(host, root)
  if (state.kind !== 'ready') return state

  const changes: GitChange[] = []
  const recreated = recreatedAfterStagedDelete(state.entries)
  for (const entry of state.entries) {
    if (recreated.has(entry.path) && entry.x === '?' && entry.y === '?') continue
    const isRecreatedDelete =
      recreated.has(entry.path) && entry.x === 'D' && entry.y === ' '
    const status = isRecreatedDelete ? 'modified' : statusFromCode(entry.x, entry.y)
    if (!status) continue
    const change: GitChange = {
      path: entry.path,
      status,
      staged: entry.x !== ' ' && entry.x !== '?' && entry.x !== '!',
    }
    if (entry.renameFrom !== undefined) change.from = entry.renameFrom
    changes.push(change)
  }
  return { kind: 'ready', changes }
}

function statusForScope(entry: PorcelainEntry, scope: GitComparisonScope): GitFileStatus | null {
  if (scope === 'uncommitted') return statusFromCode(entry.x, entry.y)
  const code = scope === 'staged' ? entry.x : entry.y
  return statusFromCode(code, code)
}

function renameCrossesScope(entry: PorcelainEntry, scope: GitComparisonScope): boolean {
  if (scope === 'staged') return entry.x === 'R'
  if (scope === 'unstaged') return entry.y === 'R'
  return entry.x === 'R' || entry.y === 'R'
}

function comparisonSources(
  entry: PorcelainEntry,
  scope: GitComparisonScope,
  status: GitFileStatus,
): { before: GitContentSource; after: GitContentSource } {
  const crossesRename = renameCrossesScope(entry, scope)
  const beforePath = crossesRename && entry.renameFrom !== undefined ? entry.renameFrom : entry.path

  if (scope === 'staged') {
    return {
      before: entry.x === 'A' ? { kind: 'empty' } : { kind: 'head', path: beforePath },
      after: entry.x === 'D' ? { kind: 'empty' } : { kind: 'index', path: entry.path },
    }
  }
  if (scope === 'unstaged') {
    return {
      before:
        status === 'untracked' || entry.y === 'A'
          ? { kind: 'empty' }
          : { kind: 'index', path: beforePath },
      after: entry.y === 'D' ? { kind: 'empty' } : { kind: 'worktree', path: entry.path },
    }
  }
  return {
    before:
      status === 'added' || status === 'untracked' || entry.x === 'A'
        ? { kind: 'empty' }
        : { kind: 'head', path: beforePath },
    after: status === 'deleted' ? { kind: 'empty' } : { kind: 'worktree', path: entry.path },
  }
}

function porcelainByPath(entries: readonly PorcelainEntry[]): Map<string, PorcelainEntry> {
  const byPath = new Map<string, PorcelainEntry>()
  for (const entry of entries) {
    const previous = byPath.get(entry.path)
    if (previous?.x === 'D' && previous.y === ' ' && entry.x === '?' && entry.y === '?') {
      byPath.set(entry.path, { path: entry.path, x: 'D', y: '?' })
      continue
    }
    byPath.set(entry.path, entry)
  }
  return byPath
}

async function uncommittedComparison(
  host: Host,
  root: string,
  prefix: string,
  porcelain: readonly PorcelainEntry[],
): Promise<GitComparisonState> {
  try {
    const head = await git(host, root, ['rev-parse', '--verify', '--quiet', 'HEAD'])
    const unborn =
      head.exit_code === 1 &&
      !head.timed_out &&
      !head.stdout_truncated &&
      !head.stderr_truncated
    if (!unborn) {
      const headFailure = execFailure(head, 'git rev-parse HEAD')
      if (headFailure !== null) return { kind: 'error', message: headFailure }
    }
    const diffOut = unborn
      ? null
      : await checkedGit(
          host,
          root,
          [
            'diff',
            '--no-ext-diff',
            '--name-status',
            '-z',
            '--find-renames',
            'HEAD',
            '--',
            '.',
          ],
          'git diff uncommitted',
        )
    const tracked = parseNameStatus(diffOut?.stdout ?? '', prefix)
    if (typeof tracked === 'string') return { kind: 'error', message: tracked }

    const stagedDeletePaths = new Set(
      porcelain
        .filter((entry) => entry.x === 'D' && entry.y === ' ')
        .map((entry) => entry.path),
    )
    const byPath = porcelainByPath(porcelain)
    const changes: GitComparisonEntry[] = tracked.map((entry) => {
      const raw = byPath.get(entry.path)
      const beforePath = entry.from ?? entry.path
      const status = raw === undefined ? entry.status : (statusFromCode(raw.x, raw.y) ?? entry.status)
      const change: GitComparisonEntry = {
        path: entry.path,
        status,
        staged: raw !== undefined && raw.x !== ' ' && raw.x !== '?' && raw.x !== '!',
        x: raw?.x ?? ' ',
        y: raw?.y ?? ' ',
        before:
          entry.status === 'added'
            ? { kind: 'empty' }
            : { kind: 'head', path: beforePath },
        after:
          entry.status === 'deleted'
            ? { kind: 'empty' }
            : { kind: 'worktree', path: entry.path },
      }
      if (entry.from !== undefined) {
        change.from = entry.from
        change.renameFrom = entry.from
      }
      return change
    })

    if (unborn) {
      for (const entry of porcelain) {
        if (entry.x === '!' || entry.y === '!' || entry.y === 'D') continue
        changes.push({
          path: entry.path,
          status: entry.x === '?' && entry.y === '?' ? 'untracked' : 'added',
          staged: entry.x !== ' ' && entry.x !== '?' && entry.x !== '!',
          x: entry.x,
          y: entry.y,
          before: { kind: 'empty' },
          after: { kind: 'worktree', path: entry.path },
        })
      }
      return { kind: 'ready', scope: 'uncommitted', changes }
    }

    // Ordinary untracked files are outside `git diff HEAD`. Recreated paths
    // already appear as deletions above; compare their exact bytes once in a
    // single batch so an identical recreation removes the deletion row.
    const recreated = new Set(
      porcelain
        .filter((entry) => entry.x === '?' && entry.y === '?')
        .map((entry) => entry.path)
        .filter((path) => stagedDeletePaths.has(path)),
    )
    for (const entry of porcelain) {
      if (entry.x !== '?' || entry.y !== '?' || recreated.has(entry.path)) continue
      changes.push({
        path: entry.path,
        status: 'untracked',
        staged: false,
        x: '?',
        y: '?',
        before: { kind: 'empty' },
        after: { kind: 'worktree', path: entry.path },
      })
    }

    if (recreated.size > 0) {
      const paths = [...recreated]
      const hashOut = await checkedGit(
        host,
        root,
        ['hash-object', '--', ...paths],
        'git hash-object recreated files',
      )
      const hashes = hashOut.stdout.trimEnd().split('\n')
      if (
        hashes.length !== paths.length ||
        hashes.some((hash) => !/^[0-9a-f]{40,64}$/i.test(hash))
      ) {
        return { kind: 'error', message: 'git hash-object returned malformed object ids' }
      }
      const headOut = await checkedGit(
        host,
        root,
        [
          'ls-tree',
          '-z',
          '--format=%(objectname)%x00%(path)',
          'HEAD',
          '--',
          ...paths.map((path) => `:(literal)${path}`),
        ],
        'git ls-tree recreated files',
      )
      if (!headOut.stdout.endsWith('\0')) {
        return { kind: 'error', message: 'git ls-tree returned incomplete object ids' }
      }
      const headFields = headOut.stdout.slice(0, -1).split('\0')
      if (headFields.length !== paths.length * 2) {
        return { kind: 'error', message: 'git ls-tree returned malformed object ids' }
      }
      const headHashes = new Map<string, string>()
      for (let i = 0; i < headFields.length; i += 2) {
        const hash = headFields[i]
        const path = rootRelative(prefix, headFields[i + 1])
        if (!/^[0-9a-f]{40,64}$/i.test(hash) || path === '') {
          return { kind: 'error', message: 'git ls-tree returned malformed object ids' }
        }
        headHashes.set(path, hash)
      }
      for (let i = 0; i < paths.length; i++) {
        const path = paths[i]
        const index = changes.findIndex((change) => change.path === path)
        const headHash = headHashes.get(path)
        if (headHash === undefined) {
          return { kind: 'error', message: 'git ls-tree omitted a recreated path' }
        }
        if (hashes[i] === headHash) {
          if (index !== -1) changes.splice(index, 1)
        } else if (index !== -1) {
          changes[index] = {
            path,
            status: 'modified',
            staged: true,
            x: 'D',
            y: '?',
            before: { kind: 'head', path },
            after: { kind: 'worktree', path },
          }
        }
      }
    }

    return { kind: 'ready', scope: 'uncommitted', changes }
  } catch (error) {
    return { kind: 'error', message: caughtGitMessage(error) }
  }
}

/** Build comparisons without reading every file up front. `uncommitted`
    compares HEAD (or empty for additions/unborn repositories) to the
    worktree; `unstaged` compares the index to the worktree; `staged`
    compares HEAD/empty to the index. Untracked files participate in the
    first two scopes. */
export async function gitComparison(
  host: Host,
  root: string,
  scope: GitComparisonScope,
): Promise<GitComparisonState> {
  const state = await porcelainStatus(host, root)
  if (state.kind !== 'ready') return state
  if (scope === 'uncommitted') {
    return uncommittedComparison(host, root, state.prefix, state.entries)
  }

  const changes: GitComparisonEntry[] = []
  for (const entry of state.entries) {
    const status = statusForScope(entry, scope)
    if (status === null || status === 'ignored') continue
    // `??` has no index side, and must not leak into the staged scope.
    if (scope === 'staged' && (entry.x === '?' || entry.x === ' ')) continue
    if (scope === 'unstaged' && entry.y === ' ') continue

    const sources = comparisonSources(entry, scope, status)
    const change: GitComparisonEntry = {
      path: entry.path,
      status,
      staged: entry.x !== ' ' && entry.x !== '?' && entry.x !== '!',
      x: entry.x,
      y: entry.y,
      ...sources,
    }
    if (entry.renameFrom !== undefined) {
      change.renameFrom = entry.renameFrom
      if (renameCrossesScope(entry, scope)) change.from = entry.renameFrom
    }
    changes.push(change)
  }
  return { kind: 'ready', scope, changes }
}

/** Resolve one side of a comparison to text. Empty sources avoid I/O;
    Git-backed sources fail rather than returning partial command output;
    worktree sources reject binary and partial `coder::read-file` responses. */
export async function gitReadSource(
  host: Host,
  root: string,
  source: GitContentSource,
): Promise<string> {
  if (source.kind === 'empty') return ''
  if (source.kind === 'worktree') {
    const out = await coderReadFile(host, joinPath(root, source.path))
    if (out.is_utf8 === false) throw new Error(`binary file: ${source.path}`)
    if (out.more_lines === true) throw new Error(`worktree read was truncated: ${source.path}`)
    return out.content ?? ''
  }

  const revision =
    source.kind === 'head'
      ? `HEAD:./${source.path}`
      : source.kind === 'index'
        ? `:./${source.path}`
        : `${source.revision}:./${source.path}`
  try {
    const out = await git(host, root, ['show', revision])
    const failure = execFailure(out, `git show ${source.kind}`)
    if (failure !== null) throw new Error(failure)
    if (out.stdout.includes('\0') || out.stdout.includes('\uFFFD')) {
      throw new Error(`binary file: ${source.path}`)
    }
    return out.stdout
  } catch (error) {
    const message = errorMessage(error)
    if (message.startsWith('git show ') || message.startsWith('binary file:')) throw error
    throw new Error(`git show ${source.kind} failed: ${message}`)
  }
}

function parseCommitSummaries(stdout: string): GitCommitSummary[] | string {
  if (stdout === '') return []
  const lines = stdout.endsWith('\n') ? stdout.slice(0, -1).split('\n') : stdout.split('\n')
  const commits: GitCommitSummary[] = []
  for (const line of lines) {
    const separator = line.indexOf('\0')
    if (separator <= 0) return 'git log returned malformed commit metadata'
    const sha = line.slice(0, separator)
    if (!/^[0-9a-f]{40,64}$/i.test(sha)) return 'git log returned an invalid commit id'
    commits.push({ sha, subject: line.slice(separator + 1) })
  }
  return commits
}

/** Recent commits from the current history. An unborn repository is a
    successful empty result, not an operational failure. */
export async function gitRecentCommits(
  host: Host,
  root: string,
  limit = 20,
): Promise<GitRecentCommitsState> {
  const repository = await probeRepository(host, root)
  if (repository.kind !== 'ready') return repository

  try {
    const head = await git(host, root, ['rev-parse', '--verify', '--quiet', 'HEAD'])
    if (
      head.exit_code === 1 &&
      !head.timed_out &&
      !head.stdout_truncated &&
      !head.stderr_truncated
    ) {
      return { kind: 'ready', commits: [] }
    }
    const headFailure = execFailure(head, 'git rev-parse HEAD')
    if (headFailure !== null) return { kind: 'error', message: headFailure }

    const count = Number.isFinite(limit)
      ? Math.max(1, Math.min(100, Math.trunc(limit)))
      : 20
    const out = await git(host, root, [
      'log',
      `--max-count=${String(count)}`,
      '--format=%H%x00%s',
    ])
    const failure = execFailure(out, 'git log')
    if (failure !== null) return { kind: 'error', message: failure }
    const commits = parseCommitSummaries(out.stdout)
    return typeof commits === 'string'
      ? { kind: 'error', message: commits }
      : { kind: 'ready', commits }
  } catch (error) {
    return { kind: 'error', message: `git metadata failed: ${errorMessage(error)}` }
  }
}

function parseRefs(stdout: string): GitRefSummary[] | string {
  if (stdout === '') return []
  const lines = stdout.endsWith('\n') ? stdout.slice(0, -1).split('\n') : stdout.split('\n')
  const refs: GitRefSummary[] = []
  for (const line of lines) {
    const fields = line.split('\0')
    if (fields.length !== 4) return 'git for-each-ref returned malformed ref metadata'
    const [fullName, sha, head, symref] = fields
    if (symref !== '') continue
    if (!/^[0-9a-f]{40,64}$/i.test(sha)) return 'git for-each-ref returned an invalid object id'

    const localPrefix = 'refs/heads/'
    const remotePrefix = 'refs/remotes/'
    const kind = fullName.startsWith(localPrefix)
      ? 'local'
      : fullName.startsWith(remotePrefix)
        ? 'remote'
        : null
    if (kind === null) return 'git for-each-ref returned an unexpected ref'
    refs.push({
      kind,
      name: fullName.slice(kind === 'local' ? localPrefix.length : remotePrefix.length),
      fullName,
      sha,
      current: head.trim() === '*',
    })
  }
  return refs
}

/** Local and remote-tracking branch refs. Symbolic aliases such as
    `refs/remotes/origin/HEAD` are omitted so menu entries are unique. */
export async function gitRefs(host: Host, root: string): Promise<GitRefsState> {
  const repository = await probeRepository(host, root)
  if (repository.kind !== 'ready') return repository

  try {
    const out = await git(host, root, [
      'for-each-ref',
      '--sort=refname',
      '--format=%(refname)%00%(objectname)%00%(HEAD)%00%(symref)',
      'refs/heads/',
      'refs/remotes/',
    ])
    const failure = execFailure(out, 'git for-each-ref')
    if (failure !== null) return { kind: 'error', message: failure }
    const refs = parseRefs(out.stdout)
    return typeof refs === 'string' ? { kind: 'error', message: refs } : { kind: 'ready', refs }
  } catch (error) {
    return { kind: 'error', message: `git metadata failed: ${errorMessage(error)}` }
  }
}

/** The committed body of one path, resolved from the directory it lives in so
    a repository nested under a non-repository root still answers. Null means
    there is no usable committed body — no repository, path untracked, binary,
    truncated, or a failed exec — never an empty string, which a caller would
    read as a real empty file. */
export async function gitHeadBaseline(
  host: Host,
  cwd: string,
  path: string,
): Promise<string | null> {
  try {
    const out = await git(host, cwd, ['show', `HEAD:./${path}`])
    if (execFailure(out, 'git show HEAD') !== null) return null
    if (out.stdout.includes('\0') || out.stdout.includes('�')) return null
    return out.stdout
  } catch {
    return null
  }
}
