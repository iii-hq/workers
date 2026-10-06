/* The branch menu's verbs that act in the folder the menu is for: checkout
   in place (a local branch, a remote one, a tag or a revision), a new branch
   checked out there, checkout-and-rebase, rebase or merge into the current
   branch, pull from a remote branch, deleting a branch on its remote, and a
   tag's push and delete. A checkout that would overwrite local changes stops
   with the files (CheckoutBlocked) for a Smart or a Force one; a remote
   branch whose local branch has commits of its own stops too
   (RemoteDiverged), to drop them or rebase them. The worktree verbs (open
   one, merge one away, push, update) stay in `worktrees.ts`. Everything goes
   through `shell::exec` in argv form. */

import type { Host } from '@iii-dev/console-ui'
import { git, gitStashPush, run } from './git-actions'

/** A rebase, a merge or a pull can take a while on a big branch. */
const LONG_MS = 120_000
/** Asks for no credentials: a remote that needs them fails at once. */
const NO_PROMPT = { env: { GIT_TERMINAL_PROMPT: '0' } }

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
}

/** A stopped merge or rebase, told in a sentence that says what to do next. */
export function conflictNote(kind: 'merge' | 'rebase', output: string): string | null {
  if (!/CONFLICT|could not apply|Resolve all conflicts|fix conflicts/i.test(output)) return null
  return kind === 'rebase'
    ? 'the rebase stopped on conflicts: resolve them, then run git rebase --continue (or --abort) in a terminal'
    : 'the merge stopped on conflicts: resolve and commit them, or run git merge --abort in a terminal'
}

/** The line of git's output that says why it failed. */
function reason(out: ExecResponse, operation: string): string {
  if (out.timed_out) return `${operation} timed out`
  const lines = `${out.stderr}\n${out.stdout}`
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
  return lines.find((line) => /^(fatal|error|hint: |!)/.test(line)) ?? lines[0] ?? `${operation} failed`
}

/** Run a merge, rebase or pull; a conflict comes back as its own message. */
async function runMerging(
  host: Host,
  cwd: string,
  args: string[],
  operation: string,
  kind: 'merge' | 'rebase',
  network = false,
): Promise<void> {
  const out = await git(host, cwd, args, LONG_MS, network ? NO_PROMPT : {})
  if (out.exit_code === 0) return
  throw new Error(conflictNote(kind, `${out.stdout}\n${out.stderr}`) ?? reason(out, operation))
}

/** `git check-ref-format --branch`: the name as git takes it, or an error. */
export async function plainName(host: Host, cwd: string, name: string): Promise<string> {
  const clean = name.trim()
  const valid = await git(host, cwd, ['check-ref-format', '--branch', clean])
  if (valid.exit_code !== 0 || valid.stdout.trim() !== clean) throw new Error(`${clean} is not a plain branch name`)
  return clean
}

/** The files git names when a checkout would overwrite them (local changes,
    and untracked files in the way); null when that is not why it stopped. */
export function overwrittenFiles(output: string): string[] | null {
  if (!/would be overwritten by checkout/.test(output)) return null
  const files: string[] = []
  let listing = false
  for (const line of output.split('\n')) {
    if (/would be overwritten by checkout:\s*$/.test(line)) listing = true
    else if (listing && line.startsWith('\t')) files.push(line.slice(1))
    else listing = false
  }
  return files
}

/** A checkout git refused: it would overwrite local changes to `files`. */
export class CheckoutBlocked extends Error {
  constructor(readonly files: string[]) {
    super(`the checkout would overwrite local changes to ${files.length} ${files.length === 1 ? 'file' : 'files'}`)
  }
}

/** A remote branch checked out over a local branch that has `ahead`
    commits it lacks: drop them, or rebase them onto it. */
export class RemoteDiverged extends Error {
  constructor(
    readonly branch: string,
    readonly remoteBranch: string,
    readonly ahead: number,
  ) {
    super(`${branch} has ${ahead} ${ahead === 1 ? 'commit' : 'commits'} ${remoteBranch} lacks`)
  }
}

/** What a checkout does with local changes it would overwrite: stop with
    CheckoutBlocked, stash them across it and bring them back (smart), or
    drop them (force). */
export type CheckoutMode = 'plain' | 'smart' | 'force'

const stashTip = async (host: Host, cwd: string) =>
  (await git(host, cwd, ['--no-optional-locks', 'rev-parse', '-q', '--verify', 'refs/stash'])).stdout.trim()

/** `git switch <args>` in place, local changes treated as `mode` says; what
    to add to the note (a stash that could not come back cleanly). */
async function switchIn(host: Host, cwd: string, args: string[], mode: CheckoutMode): Promise<string> {
  let stashed = false
  if (mode === 'smart') {
    const before = await stashTip(host, cwd)
    await gitStashPush(host, cwd, { message: `smart checkout: ${args[args.length - 1]}`, includeUntracked: true })
    stashed = (await stashTip(host, cwd)) !== before
  }
  const out = await git(host, cwd, ['switch', '--quiet', ...(mode === 'force' ? ['--force'] : []), ...args], LONG_MS)
  if (out.exit_code !== 0) {
    // Nothing moved: the changes go back where they were, or say where they wait.
    let kept = ''
    if (stashed) {
      const back = await git(host, cwd, ['stash', 'pop', '--index'])
      if (back.exit_code !== 0) kept = '; your changes stay in the newest stash: pop it in Source control'
    }
    const files = mode === 'plain' ? overwrittenFiles(`${out.stderr}\n${out.stdout}`) : null
    throw files === null ? new Error(`${reason(out, 'git switch')}${kept}`) : new CheckoutBlocked(files)
  }
  if (!stashed) return ''
  const back = await git(host, cwd, ['stash', 'pop'])
  return back.exit_code === 0
    ? ''
    : '; your changes conflict with it: resolve them in Source control (they stay in the newest stash too)'
}

/** `git switch <branch>` in place. Git refuses a branch another worktree has
    checked out, and changes that the switch would overwrite. */
export async function checkoutBranch(
  host: Host,
  cwd: string,
  branch: string,
  mode: CheckoutMode = 'plain',
): Promise<string> {
  return `checked out ${branch}${await switchIn(host, cwd, [branch], mode)}`
}

/** `origin/x` split into its remote and its branch. */
export function splitRemote(remoteBranch: string): { remote: string; branch: string } {
  const slash = remoteBranch.indexOf('/')
  return slash === -1
    ? { remote: remoteBranch, branch: '' }
    : { remote: remoteBranch.slice(0, slash), branch: remoteBranch.slice(slash + 1) }
}

/** A remote branch checked out: the local branch of its name, made to
    track it. A new one is created; an existing one with nothing of its own
    is reset to it; one with commits it lacks stops with RemoteDiverged,
    unless `resolve` says what to do with them: drop them, or rebase them
    onto it (local changes stashed across the rebase). */
export async function checkoutRemoteBranch(
  host: Host,
  cwd: string,
  remoteBranch: string,
  locals: readonly string[],
  mode: CheckoutMode = 'plain',
  resolve?: 'drop' | 'rebase',
): Promise<string> {
  const { branch } = splitRemote(remoteBranch)
  if (!locals.includes(branch)) {
    return `checked out ${branch}, tracking ${remoteBranch}${await switchIn(host, cwd, ['--track', remoteBranch], mode)}`
  }
  if (resolve === 'rebase') {
    await runMerging(host, cwd, ['rebase', '--autostash', remoteBranch, branch], 'git rebase', 'rebase')
    await run(host, cwd, ['branch', `--set-upstream-to=${remoteBranch}`, branch], 'git branch')
    return `checked out ${branch}, its commits rebased onto ${remoteBranch}, tracking it`
  }
  if (resolve !== 'drop') {
    const counts = await git(host, cwd, ['rev-list', '--left-right', '--count', `${branch}...${remoteBranch}`])
    if (counts.exit_code !== 0) throw new Error(reason(counts, 'git rev-list'))
    const [ahead = 0] = counts.stdout.trim().split(/\s+/).map(Number)
    if (ahead > 0) throw new RemoteDiverged(branch, remoteBranch, ahead)
  }
  const extra = await switchIn(host, cwd, ['-C', branch, '--track', remoteBranch], mode)
  return `checked out ${branch}, reset to ${remoteBranch}${resolve === 'drop' ? ' (its own commits dropped)' : ''}${extra}`
}

/** A tag or any revision, checked out detached in place. */
export async function checkoutRevision(
  host: Host,
  cwd: string,
  revision: string,
  mode: CheckoutMode = 'plain',
): Promise<string> {
  const rev = revision.trim()
  const known = await git(host, cwd, ['rev-parse', '--verify', '--quiet', `${rev}^{commit}`])
  if (known.exit_code !== 0) throw new Error(`${rev} names no commit`)
  return `checked out ${rev} (detached)${await switchIn(host, cwd, ['--detach', rev], mode)}`
}

/** `git switch -c <name> <start>`: a new branch, checked out in place. */
export async function newBranchHere(host: Host, cwd: string, name: string, start: string): Promise<string> {
  const branch = await plainName(host, cwd, name)
  await run(host, cwd, ['switch', '--quiet', '-c', branch, start], 'git switch -c')
  return `created and checked out ${branch} from ${start}`
}

/** `git rebase <onto> <branch>`: checks `branch` out in place, then replays
    its commits onto `onto`. */
export async function checkoutAndRebase(host: Host, cwd: string, branch: string, onto: string): Promise<string> {
  await runMerging(host, cwd, ['rebase', onto, branch], 'git rebase', 'rebase')
  return `checked out ${branch} and rebased it onto ${onto}`
}

/** A remote branch's local branch (made, tracking it, when missing)
    checked out and rebased onto `onto`. */
export async function checkoutRemoteAndRebase(
  host: Host,
  cwd: string,
  remoteBranch: string,
  onto: string,
  locals: readonly string[],
): Promise<string> {
  const { branch } = splitRemote(remoteBranch)
  if (!locals.includes(branch)) {
    await run(host, cwd, ['branch', '--track', branch, remoteBranch], 'git branch --track')
  }
  return checkoutAndRebase(host, cwd, branch, onto)
}

/** `git rebase <onto>`: the current branch replayed onto `onto`. */
export async function rebaseCurrent(host: Host, cwd: string, current: string, onto: string): Promise<string> {
  await runMerging(host, cwd, ['rebase', onto], 'git rebase', 'rebase')
  return `rebased ${current} onto ${onto}`
}

/** `git merge --no-edit <branch>` into the current branch. */
export async function mergeIntoCurrent(host: Host, cwd: string, current: string, branch: string): Promise<string> {
  await runMerging(host, cwd, ['merge', '--no-edit', branch], 'git merge', 'merge')
  return `merged ${branch} into ${current}`
}

/** `git pull` of a remote branch into the current one, by rebase or merge. */
export async function pullInto(
  host: Host,
  cwd: string,
  current: string,
  remoteBranch: string,
  rebase: boolean,
): Promise<string> {
  const { remote, branch } = splitRemote(remoteBranch)
  await runMerging(
    host,
    cwd,
    ['pull', rebase ? '--rebase' : '--no-rebase', '--no-edit', remote, branch],
    'git pull',
    rebase ? 'rebase' : 'merge',
    true,
  )
  return `pulled ${remoteBranch} into ${current} using ${rebase ? 'rebase' : 'merge'}`
}

/** `git push <remote> --delete <branch>`: the branch goes from the remote. */
export async function deleteRemoteBranch(host: Host, cwd: string, remoteBranch: string): Promise<string> {
  const { remote, branch } = splitRemote(remoteBranch)
  const out = await git(host, cwd, ['push', remote, '--delete', branch], LONG_MS, NO_PROMPT)
  if (out.exit_code !== 0) throw new Error(reason(out, 'git push --delete'))
  return `deleted ${remoteBranch}`
}

/** `git push <remote> refs/tags/<tag>`: the tag goes to the remote. */
export async function pushTag(host: Host, cwd: string, remote: string, tag: string): Promise<string> {
  const out = await git(host, cwd, ['push', remote, `refs/tags/${tag}`], LONG_MS, NO_PROMPT)
  if (out.exit_code !== 0) throw new Error(reason(out, 'git push'))
  return `pushed ${tag} to ${remote}`
}

/** `git tag -d <tag>`: the tag goes here; a remote keeps its copy. */
export async function deleteTag(host: Host, cwd: string, tag: string): Promise<string> {
  await run(host, cwd, ['tag', '-d', tag], 'git tag -d')
  return `deleted tag ${tag}`
}

/** Tags, the newest first. */
export async function readTags(host: Host, cwd: string): Promise<string[]> {
  const out = await git(host, cwd, ['for-each-ref', '--sort=-creatordate', '--format=%(refname:short)', 'refs/tags'])
  return out.exit_code === 0 ? out.stdout.split('\n').filter((name) => name !== '') : []
}

/** Remote-tracking branches, the most recently committed first (`origin/HEAD` left out). */
export function parseRemoteBranches(stdout: string): string[] {
  return stdout
    .split('\n')
    .map((line) => line.trim())
    .filter((name) => name.includes('/') && !name.endsWith('/HEAD'))
}

export async function readRemoteBranches(host: Host, cwd: string): Promise<string[]> {
  const out = await git(host, cwd, [
    'for-each-ref',
    '--sort=-committerdate',
    '--format=%(refname:short)',
    'refs/remotes',
  ])
  return out.exit_code === 0 ? parseRemoteBranches(out.stdout) : []
}

/** The branches checked out lately in this folder, newest first, from its
    reflog's `checkout: moving from A to B` entries: only branches that still
    exist, never the one checked out now. */
export function parseRecentBranches(
  reflog: string,
  locals: readonly string[],
  current: string | null,
  limit = 5,
): string[] {
  const known = new Set(locals)
  const recent: string[] = []
  for (const line of reflog.split('\n')) {
    const match = /^checkout: moving from (\S+) to (\S+)$/.exec(line.trim())
    if (!match) continue
    for (const name of [match[2], match[1]]) {
      if (name === current || !known.has(name) || recent.includes(name)) continue
      recent.push(name)
      if (recent.length === limit) return recent
    }
  }
  return recent
}

export async function readRecentReflog(host: Host, cwd: string): Promise<string> {
  const out = await git(host, cwd, ['reflog', '-n', '300', '--format=%gs', 'HEAD'])
  return out.exit_code === 0 ? out.stdout : ''
}
