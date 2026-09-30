/* Read-side git plumbing for the Commit panel: the History and Stash tabs,
   the commit box's recent messages and amend seed, and the text Generate
   hands the worker to describe. Everything goes through `shell::exec` in
   argv form, cwd-scoped to the browsed root, so nothing is shell-tokenized. */

import type { Host } from '@iii-dev/console-ui'
import { type GitComparisonEntry, type GitFileStatus, type NameStatusEntry, parseNameStatus } from './git'

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
  stdout_truncated: boolean
  stderr_truncated: boolean
}

/** Git's well-known empty tree: the "parent" of a root commit or an unborn HEAD. */
export const EMPTY_TREE = '4b825dc642cb6eb9a060e54bf8d69288fbee4904'

function git(host: Host, cwd: string, args: string[]): Promise<ExecResponse> {
  return host.iii.trigger<ExecResponse>('shell::exec', { command: 'git', args, cwd, timeout_ms: 15_000 })
}

async function run(host: Host, cwd: string, args: string[], operation: string): Promise<ExecResponse> {
  const out = await git(host, cwd, args)
  if (out.timed_out) throw new Error(`${operation} timed out`)
  if (out.exit_code !== 0) throw new Error(out.stderr.trim() || `${operation} exited ${out.exit_code}`)
  return out
}

function files(stdout: string, operation: string): NameStatusEntry[] {
  const parsed = parseNameStatus(stdout, '')
  if (typeof parsed === 'string') throw new Error(`${operation}: ${parsed}`)
  return parsed
}

/** An unborn branch answers `git log` with an error, not an empty list. */
function isUnborn(out: ExecResponse): boolean {
  return /does not have any commits yet|unknown revision|bad default revision|ambiguous argument 'HEAD'/.test(
    out.stderr,
  )
}

// ── history ──────────────────────────────────────────────────────────────

export type GitRefKind = 'head' | 'branch' | 'remote' | 'tag'

export interface GitLogRef {
  kind: GitRefKind
  name: string
}

export interface GitLogCommit {
  sha: string
  short: string
  author: string
  /** Unix seconds. */
  time: number
  parents: string[]
  refs: GitLogRef[]
  subject: string
}

/** `%D` under `--decorate=full`: `HEAD -> refs/heads/main, refs/remotes/origin/main, tag: refs/tags/v1`.
    Full names keep a local `feat/x` apart from a remote `origin/x`. */
export function parseDecorations(decoration: string): GitLogRef[] {
  const refs: GitLogRef[] = []
  for (const raw of decoration.split(', ')) {
    const item = raw.trim()
    if (item === '') continue
    if (item === 'HEAD') {
      refs.push({ kind: 'head', name: 'HEAD' })
    } else if (item.startsWith('HEAD -> refs/heads/')) {
      refs.push({ kind: 'head', name: item.slice('HEAD -> refs/heads/'.length) })
    } else if (item.startsWith('tag: refs/tags/')) {
      refs.push({ kind: 'tag', name: item.slice('tag: refs/tags/'.length) })
    } else if (item.startsWith('refs/heads/')) {
      refs.push({ kind: 'branch', name: item.slice('refs/heads/'.length) })
    } else if (item.startsWith('refs/remotes/')) {
      const name = item.slice('refs/remotes/'.length)
      // `origin/HEAD` only restates which remote branch is the default.
      if (!name.endsWith('/HEAD')) refs.push({ kind: 'remote', name })
    }
    // refs/stash and anything else is not a place a person checks out.
  }
  return refs
}

export function parseLog(stdout: string): GitLogCommit[] {
  const commits: GitLogCommit[] = []
  for (const record of stdout.split('\x1e')) {
    const trimmed = record.replace(/^\n/, '')
    if (trimmed === '') continue
    const [sha, short, author, time, parents, decoration, subject] = trimmed.split('\0')
    if (!sha || !/^[0-9a-f]{40,64}$/i.test(sha) || subject === undefined) {
      throw new Error('git log returned malformed commit metadata')
    }
    commits.push({
      sha,
      short,
      author,
      time: Number(time) || 0,
      parents: parents ? parents.split(' ').filter(Boolean) : [],
      refs: parseDecorations(decoration ?? ''),
      subject,
    })
  }
  return commits
}

/** The current branch's first-parent history under the browsed root, newest first.
    First-parent keeps the list one lane: merged branches show as their merge commit. */
export async function gitLog(host: Host, root: string, limit = 200): Promise<GitLogCommit[]> {
  const out = await git(host, root, [
    'log',
    '--first-parent',
    `--max-count=${limit}`,
    '--decorate=full',
    '--format=%H%x00%h%x00%an%x00%ct%x00%P%x00%D%x00%s%x1e',
    'HEAD',
    '--',
    '.',
  ])
  if (out.exit_code !== 0 && isUnborn(out)) return []
  if (out.timed_out) throw new Error('git log timed out')
  if (out.exit_code !== 0) throw new Error(out.stderr.trim() || `git log exited ${out.exit_code}`)
  return parseLog(out.stdout)
}

export interface GitCommitDetails {
  message: string
  email: string
  files: NameStatusEntry[]
}

/** One commit's full message and the files it changed against its first parent. */
export async function gitCommitDetails(
  host: Host,
  root: string,
  commit: Pick<GitLogCommit, 'sha' | 'parents'>,
): Promise<GitCommitDetails> {
  const [meta, diff] = await Promise.all([
    run(host, root, ['show', '-s', '--format=%ae%x00%B', commit.sha], 'git show'),
    run(
      host,
      root,
      [
        'diff',
        '--no-ext-diff',
        '--name-status',
        '-z',
        '--find-renames',
        '--relative',
        revisionParent(commit),
        commit.sha,
      ],
      'git diff',
    ),
  ])
  const separator = meta.stdout.indexOf('\0')
  return {
    email: separator === -1 ? '' : meta.stdout.slice(0, separator),
    message: (separator === -1 ? meta.stdout : meta.stdout.slice(separator + 1)).trim(),
    files: files(diff.stdout, 'git diff'),
  }
}

/** What a commit is compared against: its first parent, or the empty tree for a root commit. */
export function revisionParent(commit: Pick<GitLogCommit, 'parents'>): string {
  return commit.parents[0] ?? EMPTY_TREE
}

// ── stashes ──────────────────────────────────────────────────────────────

export interface GitStash {
  /** `stash@{n}` — shifts when an older stash is dropped; re-read after every change. */
  ref: string
  sha: string
  time: number
  /** The branch the stash was made on; null for `autostash` and detached heads. */
  branch: string | null
  message: string
}

/** `On main: message`, `WIP on main: 1a2b3c4 subject`, or a bare `autostash`. */
export function parseStashSubject(subject: string): { branch: string | null; message: string } {
  const match = /^(WIP on|On) (.+?): ([\s\S]*)$/.exec(subject)
  if (!match) return { branch: null, message: subject }
  const branch = match[2] === '(no branch)' ? null : match[2]
  const message = match[1] === 'WIP on' ? match[3].replace(/^[0-9a-f]{7,40} /, '') : match[3]
  return { branch, message }
}

export function parseStashList(stdout: string): GitStash[] {
  const stashes: GitStash[] = []
  for (const line of stdout.split('\n')) {
    if (line === '') continue
    const [ref, sha, time, subject] = line.split('\0')
    if (!ref || !sha || subject === undefined) throw new Error('git stash list returned malformed data')
    stashes.push({ ref, sha, time: Number(time) || 0, ...parseStashSubject(subject) })
  }
  return stashes
}

export async function gitStashList(host: Host, root: string): Promise<GitStash[]> {
  const out = await run(host, root, ['stash', 'list', '--format=%gd%x00%H%x00%ct%x00%gs'], 'git stash list')
  return parseStashList(out.stdout)
}

/** A file a stash records. `untracked` ones (`git stash -u`) live in the
    stash's third parent, not in its own tree. */
export interface StashFile {
  path: string
  status: GitFileStatus
  from?: string
  untracked?: boolean
}

/** The files a stash records under the root: tracked changes against the
    commit it was made on, then the unversioned files it carries, if any. */
export async function gitStashFiles(host: Host, root: string, sha: string): Promise<StashFile[]> {
  const [diff, extra] = await Promise.all([
    run(
      host,
      root,
      ['diff', '--no-ext-diff', '--name-status', '-z', '--find-renames', '--relative', `${sha}^1`, sha],
      'git diff',
    ),
    // No third parent (a stash made without -u) is an error here, meaning none.
    git(host, root, ['ls-tree', '-r', '-z', '--name-only', `${sha}^3`, '--', '.']),
  ])
  const untracked: StashFile[] =
    extra.exit_code === 0
      ? extra.stdout
          .split('\0')
          .filter(Boolean)
          .map((path) => ({ path, status: 'untracked', untracked: true }))
      : []
  return [...files(diff.stdout, 'git diff'), ...untracked]
}

/** The diff a stash file opens: its base against the stash, or, for an
    unversioned file, nothing against the stash's third parent. */
export function stashFileSource(stash: { sha: string; ref: string }, file: Pick<StashFile, 'untracked'>) {
  return file.untracked
    ? { type: 'revision' as const, from: EMPTY_TREE, to: `${stash.sha}^3`, label: stash.ref }
    : { type: 'revision' as const, from: `${stash.sha}^1`, to: stash.sha, label: stash.ref }
}

// ── the commit box ───────────────────────────────────────────────────────

/** Split `%B%x1e` records into distinct, non-empty messages, newest first. */
export function parseMessages(stdout: string, limit: number): string[] {
  const seen = new Set<string>()
  const messages: string[] = []
  for (const record of stdout.split('\x1e')) {
    const message = record.trim()
    if (message === '' || seen.has(message)) continue
    seen.add(message)
    messages.push(message)
    if (messages.length === limit) break
  }
  return messages
}

/** The messages this user committed most recently, for the recent-messages menu. */
export async function gitRecentMessages(host: Host, root: string, limit = 10): Promise<string[]> {
  const email = await git(host, root, ['config', 'user.email']).then((out) =>
    out.exit_code === 0 ? out.stdout.trim() : '',
  )
  const out = await git(host, root, [
    'log',
    `--max-count=${limit * 3}`,
    '--format=%B%x1e',
    ...(email ? ['--fixed-strings', `--author=${email}`] : []),
    'HEAD',
  ])
  if (out.exit_code !== 0) return []
  return parseMessages(out.stdout, limit)
}

/** HEAD's full message: the seed an Amend starts from. */
export async function gitHeadMessage(host: Host, root: string): Promise<string | null> {
  const out = await git(host, root, ['log', '-1', '--format=%B', 'HEAD'])
  return out.exit_code === 0 ? out.stdout.trim() || null : null
}

/** Recent subjects, so a generated message can follow the repository's style. */
export async function gitRecentSubjects(host: Host, root: string, limit = 12): Promise<string[]> {
  const out = await git(host, root, ['log', `--max-count=${limit}`, '--format=%s', 'HEAD'])
  if (out.exit_code !== 0) return []
  return out.stdout.split('\n').filter((line) => line.trim() !== '')
}

/** Cut `text` to `budget` characters, saying so. */
export function capText(text: string, budget: number): string {
  return text.length <= budget ? text : `${text.slice(0, budget)}\n[diff truncated]`
}

const MAX_NEW_FILES = 20

/** The text Generate describes: a stat and a unified diff of the selected
    tracked files against HEAD (or the empty tree before the first commit),
    followed by up to 20 selected unversioned files as additions. */
export async function gitDescribeChanges(
  host: Host,
  root: string,
  entries: readonly Pick<GitComparisonEntry, 'path' | 'status' | 'renameFrom'>[],
  budget = 60_000,
): Promise<string> {
  const tracked = entries.filter((entry) => entry.status !== 'untracked')
  const untracked = entries.filter((entry) => entry.status === 'untracked')
  const sections: string[] = []
  if (tracked.length > 0) {
    const head = await git(host, root, ['rev-parse', '--verify', '--quiet', 'HEAD'])
    const base = head.exit_code === 0 ? 'HEAD' : EMPTY_TREE
    const paths = tracked.flatMap((entry) => (entry.renameFrom ? [entry.renameFrom, entry.path] : [entry.path]))
    const [stat, patch] = await Promise.all([
      run(
        host,
        root,
        ['diff', '--no-ext-diff', '--stat=100', '--find-renames', base, '--', ...paths],
        'git diff --stat',
      ),
      run(host, root, ['diff', '--no-ext-diff', '--find-renames', '-U3', base, '--', ...paths], 'git diff'),
    ])
    sections.push(stat.stdout.trimEnd(), patch.stdout.trimEnd())
  }
  for (const entry of untracked.slice(0, MAX_NEW_FILES)) {
    // `--no-index` exits 1 when the sides differ, which a new file always does.
    const out = await git(host, root, ['diff', '--no-index', '--no-ext-diff', '--', '/dev/null', entry.path])
    if (out.exit_code === 0 || out.exit_code === 1) sections.push(out.stdout.trimEnd())
  }
  if (untracked.length > MAX_NEW_FILES) {
    sections.push(`[${untracked.length - MAX_NEW_FILES} more new files not shown]`)
  }
  return capText(sections.filter(Boolean).join('\n\n'), budget)
}
