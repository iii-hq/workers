/* Setting up a repository from the Source Control view: `git init` with a
   first commit, signing the GitHub CLI in, and `gh repo create` with the
   folder as its source. Everything goes through `shell::exec` in argv form,
   cwd-scoped to the browsed root; `gh` keeps its own login. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { coderStat, coderWriteFile, joinPath } from './coder'
import { git, run } from './git-actions'
import { isMissingFileError } from './load-error'
import { basename } from './paths'

interface ExecResponse {
  exit_code: number | null
  stdout: string
  stderr: string
  timed_out: boolean
}

/** `gh` in `cwd`. Rejects when it cannot run at all (not installed). */
export async function gh(
  host: Host,
  cwd: string,
  args: string[],
  timeoutMs = 30_000,
  stdin?: string,
): Promise<ExecResponse> {
  return host.iii.call<ExecResponse>(
    'shell::exec',
    { command: 'gh', args, cwd, timeout_ms: timeoutMs, ...(stdin === undefined ? {} : { stdin }) },
    { timeoutMs: timeoutMs + 10_000 },
  )
}

export type GhStatus =
  | { kind: 'missing' }
  | { kind: 'signed-out' }
  | { kind: 'signed-in'; login: string }
  | { kind: 'error'; message: string }

/** The account `gh auth status` reports for github.com, the active one
    first; null when it reports none. Both the current wording ("account
    x") and the older one ("as x") are read; gh has printed to either
    stream across versions. */
export function parseAuthStatus(text: string): string | null {
  const lines = text.split('\n')
  let first: string | null = null
  for (let i = 0; i < lines.length; i++) {
    const match = /Logged in to github\.com (?:account|as) ([A-Za-z0-9-]+)/.exec(lines[i])
    if (!match) continue
    first ??= match[1]
    for (let j = i + 1; j < lines.length && !/Logged in to|Failed to log in/.test(lines[j]); j++) {
      if (/Active account: true/.test(lines[j])) return match[1]
    }
  }
  return first
}

export function isProgramMissing(error: unknown): boolean {
  return /program not found|No such file or directory|ENOENT|not recognized/i.test(errorMessage(error))
}

export async function ghStatus(host: Host, cwd: string): Promise<GhStatus> {
  let out: ExecResponse
  try {
    out = await gh(host, cwd, ['auth', 'status', '--hostname', 'github.com'], 15_000)
  } catch (error) {
    return isProgramMissing(error) ? { kind: 'missing' } : { kind: 'error', message: errorMessage(error) }
  }
  if (out.timed_out) return { kind: 'error', message: "The GitHub CLI didn't respond. Check your connection, then try again." }
  const login = out.exit_code === 0 ? parseAuthStatus(`${out.stdout}\n${out.stderr}`) : null
  return login ? { kind: 'signed-in', login } : { kind: 'signed-out' }
}

/** Typed into a terminal: the browser sign-in, Git over HTTPS through gh. */
export const GH_LOGIN_COMMAND = 'gh auth login --hostname github.com --git-protocol https --web'

/** A classic token with the scopes `gh` needs to create and push a repository. */
export const GH_TOKEN_URL =
  'https://github.com/settings/tokens/new?description=iii%20IDE&scopes=repo,read:org,workflow'

export const GH_INSTALL_URL = 'https://cli.github.com'

export interface InstallHint {
  command: string | null
  label: string
}

/** How to install gh on this machine, from the browser's own platform. */
export function ghInstallHint(platform: string): InstallHint {
  if (/mac/i.test(platform)) return { command: 'brew install gh', label: 'Homebrew' }
  if (/win/i.test(platform)) return { command: 'winget install --id GitHub.cli', label: 'winget' }
  return { command: null, label: 'your package manager' }
}

/** Signs gh in with a pasted token, then lets git push through it. */
export async function ghSignInWithToken(host: Host, cwd: string, token: string): Promise<string> {
  const out = await gh(host, cwd, ['auth', 'login', '--hostname', 'github.com', '--git-protocol', 'https', '--with-token'], 30_000, `${token.trim()}\n`)
  if (out.exit_code !== 0) {
    const detail = out.stderr.trim()
    throw new Error(
      /missing required scope/i.test(detail)
        ? 'This token is missing the repo or read:org scope. Create one with both, then paste it again.'
        : "GitHub didn't accept this token. Check you copied all of it, then paste it again.",
    )
  }
  await gh(host, cwd, ['auth', 'setup-git', '--hostname', 'github.com'], 15_000).catch(() => undefined)
  const status = await ghStatus(host, cwd)
  if (status.kind !== 'signed-in') throw new Error("The GitHub CLI still isn't signed in. Try again.")
  return status.login
}

/** A GitHub name for the folder: GitHub allows letters, digits, `.`, `-`
    and `_`; anything else becomes `-`. */
export function suggestRepoName(root: string): string {
  const name = basename(root.replace(/\/+$/, ''))
    .replace(/[^A-Za-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return name === '' || name === '.' || name === '..' ? 'my-project' : name.slice(0, 100)
}

/** Why GitHub would refuse `name`, or null. */
export function repoNameProblem(name: string): string | null {
  const trimmed = name.trim()
  if (trimmed === '') return 'Enter a name for the repository.'
  if (trimmed.length > 100) return 'Use 100 characters or fewer.'
  if (trimmed === '.' || trimmed === '..') return 'GitHub reserves this name. Choose another.'
  if (!/^[A-Za-z0-9._-]+$/.test(trimmed)) return 'Use only letters, numbers, dots, dashes and underscores.'
  return null
}

/** Written when the folder has no .gitignore, so a first commit skips
    dependencies, build output and local secrets. */
export const STARTER_GITIGNORE = `.DS_Store
node_modules/
dist/
build/
target/
.venv/
__pycache__/
.env
.env.*
!.env.example
`

export async function hasGitignore(host: Host, root: string): Promise<boolean> {
  try {
    await coderStat(host, joinPath(root, '.gitignore'))
    return true
  } catch (error) {
    // The bus rejects with the handler's error body, not an Error.
    if (isMissingFileError(errorMessage(error))) return false
    throw error
  }
}

/** Git has no name or email to put on a commit. */
export class IdentityMissingError extends Error {
  constructor() {
    super('Git needs your name and email before it can commit')
  }
}

export function isIdentityMissing(stderr: string): boolean {
  return /Please tell me who you are|empty ident name|unable to auto-detect email/i.test(stderr)
}

export interface InitResult {
  committed: boolean
  wroteGitignore: boolean
}

/** `git init` on `main`, then (if asked) every file in one first commit.
    A folder with nothing in it stays without a commit. */
export async function gitInitRepository(
  host: Host,
  root: string,
  options: { gitignore: boolean; commit: boolean },
): Promise<InitResult> {
  const init = await git(host, root, ['init', '-b', 'main'])
  if (init.exit_code !== 0) {
    // Git before 2.28 has no -b: start the unborn branch on main by hand.
    await run(host, root, ['init'], 'git init')
    await run(host, root, ['symbolic-ref', 'HEAD', 'refs/heads/main'], 'git symbolic-ref')
  }
  let wroteGitignore = false
  if (options.gitignore && !(await hasGitignore(host, root))) {
    const result = await coderWriteFile(host, joinPath(root, '.gitignore'), STARTER_GITIGNORE)
    if (!result.success) throw new Error(result.error?.message ?? 'could not write .gitignore')
    wroteGitignore = true
  }
  const committed = options.commit ? await gitInitialCommit(host, root) : false
  return { committed, wroteGitignore }
}

/** Every file in one commit; false when there was nothing to commit. */
export async function gitInitialCommit(host: Host, root: string): Promise<boolean> {
  await run(host, root, ['add', '-A', '--', '.'], 'git add', 120_000)
  const staged = await git(host, root, ['diff', '--cached', '--quiet'])
  if (staged.exit_code === 0) return false
  const out = await git(host, root, ['commit', '-q', '-m', 'Initial commit'], 60_000)
  if (out.exit_code === 0) return true
  if (isIdentityMissing(out.stderr)) throw new IdentityMissingError()
  throw new Error(out.stderr.trim() || `git commit exited ${out.exit_code}`)
}

/** A commit identity from the signed-in GitHub account: its name (or
    login) and its private noreply address, so no real email is published. */
export async function ghIdentity(host: Host, cwd: string): Promise<{ name: string; email: string } | null> {
  try {
    const out = await gh(host, cwd, ['api', 'user', '--jq', '[.id, .login, .name // ""] | @tsv'], 15_000)
    if (out.exit_code !== 0) return null
    const [id, login, name] = out.stdout.trim().split('\t')
    if (!id || !login) return null
    return { name: name || login, email: `${id}+${login}@users.noreply.github.com` }
  } catch {
    return null
  }
}

/** Saves the commit identity in the global Git config. */
export async function gitSetIdentity(host: Host, root: string, name: string, email: string): Promise<void> {
  await run(host, root, ['config', '--global', 'user.name', name.trim()], 'git config')
  await run(host, root, ['config', '--global', 'user.email', email.trim()], 'git config')
}

/** The repository has a commit to push. */
export async function gitHasCommits(host: Host, root: string): Promise<boolean> {
  const out = await git(host, root, ['rev-parse', '--verify', '--quiet', 'HEAD'])
  return out.exit_code === 0
}

export async function gitRemotes(host: Host, root: string): Promise<string[]> {
  const out = await git(host, root, ['remote'])
  if (out.exit_code !== 0) return []
  return out.stdout.split('\n').map((line) => line.trim()).filter(Boolean)
}

export type Visibility = 'private' | 'public'

/** The signed-in account already has a repository with this name. */
export class NameTakenError extends Error {
  constructor(name: string) {
    super(`You already have a repository named ${name}. Choose another name.`)
  }
}

export interface CreateRepoRequest {
  name: string
  description: string
  visibility: Visibility
}

export interface CreateRepoResult {
  url: string | null
  /** The repository exists but the first push did not go through. */
  pushError: string | null
}

/** The repository's web address in gh's output. */
export function parseRepoUrl(text: string): string | null {
  const match = /https:\/\/github\.com\/[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+?(?=\.git\b|[\s)]|$)/m.exec(text)
  return match ? match[0] : null
}

/** `gh repo create` with the folder as its source: adds `origin` and,
    when there is a commit, pushes the current branch. */
export async function ghCreateRepository(host: Host, root: string, request: CreateRepoRequest): Promise<CreateRepoResult> {
  const push = await gitHasCommits(host, root)
  const args = [
    'repo',
    'create',
    request.name.trim(),
    `--${request.visibility}`,
    '--source',
    '.',
    '--remote',
    'origin',
    ...(push ? ['--push'] : []),
    ...(request.description.trim() ? ['--description', request.description.trim()] : []),
  ]
  const out = await gh(host, root, args, 120_000)
  const text = `${out.stdout}\n${out.stderr}`
  if (out.exit_code === 0) return { url: parseRepoUrl(text) ?? (await originUrl(host, root)), pushError: null }
  // Created and wired to origin, then the push failed: the repository is
  // there, only the push is left to do.
  if ((await gitRemotes(host, root)).includes('origin')) {
    return { url: (await originUrl(host, root)) ?? parseRepoUrl(text), pushError: out.stderr.trim() || 'git push failed' }
  }
  if (out.timed_out) throw new Error('gh repo create timed out')
  const detail = out.stderr.trim()
  if (/Name already exists/i.test(detail)) throw new NameTakenError(request.name.trim())
  throw new Error(detail || `gh repo create exited ${out.exit_code}`)
}

/** `origin`'s web address, without the `.git`. */
async function originUrl(host: Host, root: string): Promise<string | null> {
  const out = await git(host, root, ['remote', 'get-url', 'origin'])
  if (out.exit_code !== 0) return null
  const url = out.stdout.trim()
  const ssh = /^git@github\.com:(.+?)(?:\.git)?$/.exec(url)
  if (ssh) return `https://github.com/${ssh[1]}`
  return url.replace(/\.git$/, '') || null
}
