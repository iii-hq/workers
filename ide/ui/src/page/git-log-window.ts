/* The reads behind the Git window's Log: the refs (local and remote
   branches, tags, with their upstream counts and worktrees), the commit log
   one page at a time, and the details of one commit. Everything goes
   through `shell::exec` in argv form; records are framed by NUL, and a
   read the worker's output cap cut short keeps its complete records only.

   The log is read from the tips a refs read fixed, fed on stdin: "all"
   means every ref's commit plus HEAD, a branch its own commit. So `--skip`
   stays exact across pages even when a ref moves in between, and the ref
   chips come from the same read as the rows they label. */

import type { Host } from '@iii-dev/console-ui'
import { type NameStatusEntry, parseNameStatus } from './git'
import { git } from './git-actions'
import { groupBranches } from './worktrees'

/** A branch or tag. `name` is short: `main`, `origin/main`, `v1.0`. */
export interface LogRef {
  kind: 'local' | 'remote' | 'tag'
  name: string
  /** `refs/heads/main`. */
  fullName: string
  /** The commit it names; an annotated tag's own commit. */
  sha: string
  /** HEAD is this branch. */
  current: boolean
  /** Its upstream (`origin/main`), and how far it is from it. */
  upstream?: string
  /** Commits it has that its upstream lacks. */
  ahead?: number
  /** Commits its upstream has that it lacks. */
  behind?: number
  /** Its upstream is configured but gone. */
  gone?: boolean
  /** The worktree it is checked out in. */
  worktree?: string
  /** Its tip's subject, and when that was committed (unix seconds). */
  subject?: string
  date?: number
}

export interface RefsSnapshot {
  refs: LogRef[]
  /** HEAD's commit; null while it is unborn (no commits yet). */
  head: string | null
  shallow: boolean
  /** The browsed root below the repository's top level, with a trailing
      slash; '' at the top. */
  prefix: string
  /** The listing was cut short (tags go first: they sort last). */
  truncated: boolean
  /** The raw listing and HEAD: when it is unchanged, nothing moved. */
  signature: string
}

export interface LogCommit {
  sha: string
  parents: string[]
  author: string
  email: string
  /** Author date, unix seconds. */
  date: number
  subject: string
}

export interface LogFilter {
  /** Matched against commit messages; a hash prefix also finds its commit. */
  text?: string
  /** `text` is a regular expression rather than a literal. */
  regex?: boolean
  caseSensitive?: boolean
  author?: string
  /** Unix seconds. */
  since?: number
  /** How `since` was picked ("Last 7 days"), for its label. Display only. */
  sinceLabel?: string
  until?: number
  /** Repository-relative paths. */
  paths?: string[]
  /** Only the history this commit (a full hash) reaches: "History up to
      here". It stands in for the branch the log follows. */
  upTo?: string
}

export interface CommitFile extends NameStatusEntry {
  /** `path` below the browsed root; null when the file lies outside it. */
  rel: string | null
  /** `path` as seen from the browsed root, up with `../` when it lies
      outside: what the diff tab of this change opens. */
  view: string
}

/** A repository path as seen from the folder `prefix` ('' at the top, else
    ending in '/'): below it, or climbing out with `../`. */
export function fromFolder(prefix: string, path: string): string {
  const from = prefix.split('/').filter(Boolean)
  const to = path.split('/')
  let common = 0
  while (common < from.length && common < to.length - 1 && from[common] === to[common]) common += 1
  return [...from.slice(common).map(() => '..'), ...to.slice(common)].join('/')
}

export interface CommitDetails {
  sha: string
  parents: string[]
  author: string
  authorEmail: string
  authorDate: number
  committer: string
  committerEmail: string
  committerDate: number
  /** The whole message. */
  message: string
  /** What `%G?` says: G good, B bad, U good but untrusted, N none, and so on. */
  signature: string
  /** The changes against the first parent (everything for a root commit),
      paths relative to the repository's top level. */
  files: CommitFile[]
  /** The message or the file list was cut short. */
  truncated: boolean
}

/** A node of the branches tree. */
export type RefTreeNode =
  | { kind: 'head'; id: 'head'; label: string; sha: string | null; ref: LogRef | null }
  | { kind: 'section'; id: string; label: string; children: RefTreeNode[] }
  | { kind: 'folder'; id: string; label: string; children: RefTreeNode[] }
  | { kind: 'ref'; id: string; label: string; ref: LogRef }

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
  stdout_truncated: boolean
}

const LOG_FORMAT = '--format=%H%x00%P%x00%aN%x00%aE%x00%at%x00%s%x00'
const LOG_FIELDS = 6
const REF_FORMAT =
  '--format=%(refname)%00%(if)%(*objectname)%(then)%(*objectname)%(else)%(objectname)%(end)%00%(HEAD)%00' +
  '%(symref)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(worktreepath)%00%(contents:subject)%00' +
  '%(committerdate:unix)%00'
const REF_FIELDS = 9
const HEX = /^[0-9a-f]{4,64}$/i

function failed(out: ExecResponse, operation: string): Error {
  if (out.timed_out) return new Error(`${operation} timed out`)
  return new Error(out.stderr.trim().split('\n').slice(-1)[0] || `${operation} exited ${out.exit_code}`)
}

/** The complete records of `stdout`: `fields` NUL-ended values, then a
    newline. A tail the output cap cut is left out. */
export function parseRecords(stdout: string, fields: number): string[][] {
  const pieces = stdout.split('\0\n')
  // The last piece is '' after a complete listing, or what the cap cut.
  pieces.pop()
  return pieces.map((piece) => piece.split('\0')).filter((values) => values.length === fields)
}

function shortName(fullName: string): string {
  return fullName.replace(/^refs\/(heads|remotes|tags)\//, '')
}

function parseTrack(track: string): Pick<LogRef, 'ahead' | 'behind' | 'gone'> {
  if (track === 'gone') return { gone: true }
  const ahead = /ahead (\d+)/.exec(track)
  const behind = /behind (\d+)/.exec(track)
  return {
    ...(ahead ? { ahead: Number(ahead[1]) } : {}),
    ...(behind ? { behind: Number(behind[1]) } : {}),
  }
}

/** Parses the refs listing; symbolic refs (`origin/HEAD`) are left out. */
export function parseRefs(stdout: string): LogRef[] {
  const refs: LogRef[] = []
  for (const [fullName, sha, head, symref, upstream, track, worktree, subject, date] of parseRecords(
    stdout,
    REF_FIELDS,
  )) {
    if (symref !== '') continue
    const kind = fullName.startsWith('refs/heads/') ? 'local' : fullName.startsWith('refs/remotes/') ? 'remote' : 'tag'
    refs.push({
      kind,
      name: shortName(fullName),
      fullName,
      sha,
      current: head === '*',
      ...(upstream !== '' ? { upstream, ...parseTrack(track) } : {}),
      ...(worktree !== '' ? { worktree } : {}),
      ...(subject !== '' ? { subject } : {}),
      ...(date !== '' ? { date: Number(date) } : {}),
    })
  }
  return refs
}

/** Every ref of the repository `root` is in, with HEAD, or null when it is
    not in one. */
export async function readRefs(host: Host, root: string): Promise<RefsSnapshot | null> {
  const [probe, listing] = await Promise.all([
    git(host, root, ['rev-parse', '--is-shallow-repository', '--show-prefix', '--verify', '-q', 'HEAD']),
    git(host, root, ['for-each-ref', '--sort=refname', REF_FORMAT, 'refs/heads', 'refs/remotes', 'refs/tags']),
  ])
  if (probe.exit_code === 128) {
    if (/not a git repository/i.test(probe.stderr)) return null
    throw failed(probe, 'git rev-parse')
  }
  // An unborn HEAD fails `--verify` (exit 1) after the other answers.
  if (probe.exit_code !== 0 && probe.exit_code !== 1) throw failed(probe, 'git rev-parse')
  if (listing.exit_code !== 0) throw failed(listing, 'git for-each-ref')
  const [shallow = 'false', prefix = '', head = ''] = probe.stdout.split('\n')
  const sha = probe.exit_code === 0 && HEX.test(head) ? head : null
  return {
    refs: parseRefs(listing.stdout),
    head: sha,
    shallow: shallow === 'true',
    prefix,
    truncated: listing.stdout_truncated,
    signature: `${sha ?? ''}\n${listing.stdout}`,
  }
}

/** The branches tree: HEAD, then local branches (the default one first)
    and each remote's, grouped by the part before their first `/`, then the
    tags. */
export function refsTree(snapshot: RefsSnapshot, defaultBranch: string | null = null): RefTreeNode[] {
  const byKind = (kind: LogRef['kind']) => snapshot.refs.filter((ref) => ref.kind === kind)
  const grouped = (refs: LogRef[], idBase: string, strip: string): RefTreeNode[] =>
    groupBranches(refs.map((ref) => ({ name: ref.name.slice(strip.length), ref }))).map(
      (group): RefTreeNode =>
        group.prefix === null
          ? {
              kind: 'ref',
              id: `ref:${group.branches[0].ref.fullName}`,
              label: group.branches[0].name,
              ref: group.branches[0].ref,
            }
          : {
              kind: 'folder',
              id: `${idBase}/${group.prefix}`,
              label: group.prefix,
              children: group.branches.map(
                ({ name, ref }): RefTreeNode => ({
                  kind: 'ref',
                  id: `ref:${ref.fullName}`,
                  label: name.slice(name.indexOf('/') + 1),
                  ref,
                }),
              ),
            },
    )
  const local = byKind('local').sort((a, b) => Number(b.name === defaultBranch) - Number(a.name === defaultBranch))
  const remotes = new Map<string, LogRef[]>()
  for (const ref of byKind('remote')) {
    const remote = ref.name.split('/')[0]
    remotes.set(remote, [...(remotes.get(remote) ?? []), ref])
  }
  const current = snapshot.refs.find((ref) => ref.current) ?? null
  return [
    { kind: 'head', id: 'head', label: current ? `HEAD (${current.name})` : 'HEAD', sha: snapshot.head, ref: current },
    { kind: 'section', id: 'local', label: 'Local', children: grouped(local, 'local', '') },
    {
      kind: 'section',
      id: 'remote',
      label: 'Remote',
      children: [...remotes].map(
        ([remote, refs]): RefTreeNode => ({
          kind: 'folder',
          id: `remote/${remote}`,
          label: remote,
          children: grouped(refs, `remote/${remote}`, `${remote}/`),
        }),
      ),
    },
    {
      kind: 'section',
      id: 'tag',
      label: 'Tags',
      children: byKind('tag').map(
        (ref): RefTreeNode => ({ kind: 'ref', id: `ref:${ref.fullName}`, label: ref.name, ref }),
      ),
    },
  ]
}

/** The refs on each commit, for the log's chips: local branches (the
    current one first), then remote branches, then tags. */
export function labelsBySha(snapshot: RefsSnapshot): Map<string, LogRef[]> {
  const order = (ref: LogRef) => (ref.kind === 'local' ? (ref.current ? 0 : 1) : ref.kind === 'remote' ? 2 : 3)
  const labels = new Map<string, LogRef[]>()
  for (const ref of [...snapshot.refs].sort((a, b) => order(a) - order(b))) {
    labels.set(ref.sha, [...(labels.get(ref.sha) ?? []), ref])
  }
  return labels
}

/** The commits the log starts from: every ref and HEAD, or one ref's. */
export function logTips(snapshot: RefsSnapshot, fullName: string | null): string[] {
  if (fullName !== null) {
    if (fullName === 'HEAD') return snapshot.head === null ? [] : [snapshot.head]
    const ref = snapshot.refs.find((candidate) => candidate.fullName === fullName)
    return ref ? [ref.sha] : []
  }
  const tips = new Set(snapshot.refs.map((ref) => ref.sha))
  if (snapshot.head !== null) tips.add(snapshot.head)
  return [...tips]
}

/** Only the branch and path filters keep each commit's parents in the
    result; the others leave rows whose lines would lead nowhere. */
export function showsGraph(filter: LogFilter): boolean {
  return !filter.text && !filter.author && filter.since === undefined && filter.until === undefined
}

/** `git log` for one page, reading its tips from stdin. */
export function logArgs(filter: LogFilter, skip: number, limit: number): string[] {
  const args = ['-c', 'log.follow=false', 'log', '--stdin', '--topo-order', '--no-color', '--no-show-signature']
  args.push(LOG_FORMAT, `--skip=${skip}`, `--max-count=${limit}`)
  if (filter.text) {
    args.push(`--grep=${filter.text}`, filter.regex ? '--extended-regexp' : '--fixed-strings')
    if (!filter.caseSensitive) args.push('--regexp-ignore-case')
  }
  // A name picked from the log matches as written: every limiting pattern is
  // a fixed string, or an extended regex beside a regex text filter.
  if (filter.author) {
    if (!filter.text) args.push('--fixed-strings')
    args.push(
      `--author=${filter.text && filter.regex ? filter.author.replace(/[\\^$.*+?()[\]{}|]/g, '\\$&') : filter.author}`,
    )
  }
  if (filter.since !== undefined) args.push(`--since=@${filter.since}`)
  if (filter.until !== undefined) args.push(`--until=@${filter.until}`)
  // Parents rewritten to the commits that touch the paths keep the lines joined.
  if (filter.paths && filter.paths.length > 0) args.push('--parents', '--', ...filter.paths)
  return args
}

function toCommit([sha, parents, author, email, date, subject]: string[]): LogCommit {
  return { sha, parents: parents === '' ? [] : parents.split(' '), author, email, date: Number(date), subject }
}

/** One page of the log from `tips`: at most `limit` commits after the
    first `skip`. `done` once the history ran out. */
export async function readLogPage(
  host: Host,
  root: string,
  tips: readonly string[],
  filter: LogFilter,
  skip: number,
  limit = 1000,
): Promise<{ commits: LogCommit[]; done: boolean }> {
  if (tips.length === 0) return { commits: [], done: true }
  const out = await git(host, root, logArgs(filter, skip, limit), 30_000, { stdin: `${tips.join('\n')}\n` })
  // A path filter naming nothing git knows answers 128 like a bad revision.
  if (out.exit_code !== 0) throw failed(out, 'git log')
  const commits = parseRecords(out.stdout, LOG_FIELDS).map(toCommit)
  if (out.stdout_truncated && commits.length === 0) throw new Error('a commit is larger than the shell output cap')
  return { commits, done: !out.stdout_truncated && commits.length < limit }
}

/** The commit a hash prefix names, or null. */
export async function findCommit(host: Host, root: string, text: string): Promise<LogCommit | null> {
  if (!HEX.test(text)) return null
  const out = await git(host, root, [
    'log',
    '--no-walk',
    '--no-color',
    '--no-show-signature',
    LOG_FORMAT,
    '--end-of-options',
    `${text}^{commit}`,
    '--',
  ])
  if (out.exit_code !== 0) return null
  return parseRecords(out.stdout, LOG_FIELDS).map(toCommit)[0] ?? null
}

/** The changed files of a name-status listing the cap cut short: the
    records that parse, dropping what the cut left of the last one. */
function filesOf(stdout: string, truncated: boolean): NameStatusEntry[] {
  const whole = parseNameStatus(stdout, '')
  if (typeof whole !== 'string') return whole
  if (!truncated) throw new Error(whole)
  let text = stdout.slice(0, stdout.lastIndexOf('\0') + 1)
  // A rename record is three fields: at most two more to drop.
  for (let drop = 0; drop < 3; drop += 1) {
    const parsed = parseNameStatus(text, '')
    if (typeof parsed !== 'string') return parsed
    text = text.slice(0, text.lastIndexOf('\0', text.length - 2) + 1)
  }
  return []
}

/** One commit's message, people, signature status and changed files.
    `prefix` is the browsed root below the top level (see RefsSnapshot). */
/** Seen from the browsed folder: below it (`rel`), and how to reach it (`view`). */
function placed(prefix: string, file: NameStatusEntry): CommitFile {
  return {
    ...file,
    rel: file.path.startsWith(prefix) ? file.path.slice(prefix.length) : null,
    view: fromFolder(prefix, file.path),
  }
}

/** The files that differ between `ref` and the working tree, staged or
    not: WebStorm's "Show Diff with Working Tree". Untracked files are left
    out, as `git diff` leaves them. */
export async function readWorkingDiff(
  host: Host,
  root: string,
  prefix: string,
  ref: string,
): Promise<{ files: CommitFile[]; truncated: boolean }> {
  const out = await git(host, root, ['diff', '--name-status', '-z', '-M', '--no-color', '--end-of-options', ref, '--'])
  if (out.exit_code !== 0) throw failed(out, 'git diff')
  return {
    files: filesOf(out.stdout, out.stdout_truncated).map((file) => placed(prefix, file)),
    truncated: out.stdout_truncated,
  }
}

export async function readCommitDetails(host: Host, root: string, prefix: string, sha: string): Promise<CommitDetails> {
  const [meta, tree] = await Promise.all([
    git(host, root, [
      'log',
      '-1',
      '--no-color',
      '--no-show-signature',
      '--format=%H%x00%P%x00%aN%x00%aE%x00%at%x00%cN%x00%cE%x00%ct%x00%G?%x00%B',
      '--end-of-options',
      sha,
      '--',
    ]),
    git(host, root, [
      'diff-tree',
      '-r',
      '-z',
      '--name-status',
      '-M',
      '--root',
      '--no-commit-id',
      '--diff-merges=first-parent',
      '--end-of-options',
      sha,
    ]),
  ])
  if (meta.exit_code !== 0) throw failed(meta, 'git log')
  if (tree.exit_code !== 0) throw failed(tree, 'git diff-tree')
  const [full, parents, author, authorEmail, authorDate, committer, committerEmail, committerDate, signature, ...body] =
    meta.stdout.split('\0')
  if (full === undefined || !HEX.test(full)) throw new Error(`${sha} is not a commit`)
  const files = filesOf(tree.stdout, tree.stdout_truncated).map((file) => placed(prefix, file))
  return {
    sha: full,
    parents: parents === '' ? [] : parents.split(' '),
    author,
    authorEmail,
    authorDate: Number(authorDate),
    committer,
    committerEmail,
    committerDate: Number(committerDate),
    message: body.join('\0').replace(/\n+$/, ''),
    signature,
    files,
    truncated: meta.stdout_truncated || tree.stdout_truncated,
  }
}

/** The branches, local and remote, that have `sha`: the first `cap` names
    and how many there are. */
export async function readContainingBranches(
  host: Host,
  root: string,
  sha: string,
  cap = 20,
): Promise<{ names: string[]; total: number; partial: boolean }> {
  const out = await git(
    host,
    root,
    ['for-each-ref', '--contains', sha, '--format=%(refname)%00%(symref)%00', 'refs/heads', 'refs/remotes'],
    15_000,
  )
  if (out.exit_code !== 0) throw failed(out, 'git for-each-ref --contains')
  const names = parseRecords(out.stdout, 2)
    .filter(([, symref]) => symref === '')
    .map(([fullName]) => shortName(fullName))
  return { names: names.slice(0, cap), total: names.length, partial: out.stdout_truncated }
}

/** A long branch name cut in the middle, keeping its folder and its last
    word: `fix/supply-chain-hardening` reads `fix/…-chain-hardening`. */
export function middle(name: string, max = 22): string {
  if (name.length <= max) return name
  const slash = name.indexOf('/')
  const head = slash > 0 && slash < 10 ? name.slice(0, slash + 1) : name.slice(0, 4)
  const room = max - head.length - 1
  // The longest tail that fits and starts at a word break.
  let start = name.length - room
  for (let at = head.length; at < name.length; at += 1) {
    if ((name[at] === '-' || name[at] === '/') && name.length - at <= room) {
      start = at
      break
    }
  }
  return `${head}…${name.slice(start)}`
}
