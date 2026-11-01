import { describe, expect, it, vi } from 'vitest'
import {
  ghCreateRepository,
  ghInstallHint,
  ghStatus,
  gitInitialCommit,
  IdentityMissingError,
  NameTakenError,
  parseAuthStatus,
  parseRepoUrl,
  repoNameProblem,
  suggestRepoName,
} from '../repo-setup'

type Exec = { exit_code: number | null; stdout: string; stderr: string; timed_out: boolean }
const ok = (stdout = ''): Exec => ({ exit_code: 0, stdout, stderr: '', timed_out: false })
const fail = (stderr: string, code = 1): Exec => ({ exit_code: code, stdout: '', stderr, timed_out: false })

/** A host whose `shell::exec` answers from `answer(command, args)`. */
function hostWith(answer: (command: string, args: string[]) => Exec | Error) {
  const calls: string[][] = []
  const call = vi.fn(async (_fn: string, payload: { command: string; args: string[] }) => {
    calls.push([payload.command, ...payload.args])
    const out = answer(payload.command, payload.args)
    if (out instanceof Error) throw out
    return out
  })
  return { host: { iii: { call } } as never, calls }
}

describe('parseAuthStatus', () => {
  it('reads the current wording and prefers the active account', () => {
    const text = [
      'github.com',
      '  ✓ Logged in to github.com account work (keyring)',
      '  - Active account: false',
      '  ✓ Logged in to github.com account ada (keyring)',
      '  - Active account: true',
    ].join('\n')
    expect(parseAuthStatus(text)).toBe('ada')
  })

  it('reads the older "as" wording', () => {
    expect(parseAuthStatus('github.com\n  ✓ Logged in to github.com as octocat (oauth_token)')).toBe('octocat')
  })

  it('reports nobody when signed out', () => {
    expect(parseAuthStatus('You are not logged into any GitHub hosts. To log in, run: gh auth login')).toBeNull()
  })
})

describe('ghStatus', () => {
  it('is missing when gh cannot be spawned', async () => {
    const { host } = hostWith(() => new Error('spawn "gh": No such file or directory — program not found; is it installed and on PATH?'))
    expect(await ghStatus(host, '/p')).toEqual({ kind: 'missing' })
  })

  it('is signed out on a non-zero exit', async () => {
    const { host } = hostWith(() => fail('You are not logged into any GitHub hosts.'))
    expect(await ghStatus(host, '/p')).toEqual({ kind: 'signed-out' })
  })

  it('names the account when signed in', async () => {
    const { host } = hostWith(() => ok('github.com\n  ✓ Logged in to github.com account ada (keyring)\n  - Active account: true'))
    expect(await ghStatus(host, '/p')).toEqual({ kind: 'signed-in', login: 'ada' })
  })
})

describe('repository names', () => {
  it('suggests a GitHub-safe name from the folder', () => {
    expect(suggestRepoName('/Users/ada/My Project!')).toBe('My-Project')
    expect(suggestRepoName('/Users/ada/app/')).toBe('app')
    expect(suggestRepoName('/')).toBe('my-project')
  })

  it('explains what GitHub would refuse', () => {
    expect(repoNameProblem('ok-name_1.2')).toBeNull()
    expect(repoNameProblem('')).toMatch(/name/)
    expect(repoNameProblem('has space')).toMatch(/letters/)
    expect(repoNameProblem('..')).toMatch(/reserves/)
  })
})

describe('parseRepoUrl', () => {
  it('finds the web address in gh output, without .git', () => {
    expect(parseRepoUrl('https://github.com/ada/app\n')).toBe('https://github.com/ada/app')
    expect(parseRepoUrl('✓ Added remote https://github.com/ada/app.git')).toBe('https://github.com/ada/app')
    expect(parseRepoUrl('nothing here')).toBeNull()
  })
})

describe('ghInstallHint', () => {
  it('picks a package manager from the platform', () => {
    expect(ghInstallHint('MacIntel').command).toBe('brew install gh')
    expect(ghInstallHint('Win32').command).toMatch(/winget/)
    expect(ghInstallHint('Linux x86_64').command).toBeNull()
  })
})

describe('gitInitialCommit', () => {
  it('commits nothing when nothing is staged', async () => {
    const { host, calls } = hostWith((_c, args) => ok(args[0] === 'diff' ? '' : ''))
    expect(await gitInitialCommit(host, '/p')).toBe(false)
    expect(calls.some((call) => call[1] === 'commit')).toBe(false)
  })

  it('asks for an identity when git has none', async () => {
    const { host } = hostWith((_c, args) =>
      args[0] === 'diff' ? fail('', 1) : args[0] === 'commit' ? fail('*** Please tell me who you are.', 128) : ok(),
    )
    await expect(gitInitialCommit(host, '/p')).rejects.toBeInstanceOf(IdentityMissingError)
  })
})

describe('ghCreateRepository', () => {
  it('pushes when there is a commit and returns the address', async () => {
    const { host, calls } = hostWith((command, args) =>
      command === 'git' ? ok('abc') : args[0] === 'repo' ? ok('https://github.com/ada/app\n') : ok(),
    )
    const result = await ghCreateRepository(host, '/p', { name: 'app', description: 'Hi', visibility: 'private' })
    expect(result).toEqual({ url: 'https://github.com/ada/app', pushError: null })
    expect(calls.find((call) => call[0] === 'gh')).toEqual([
      'gh', 'repo', 'create', 'app', '--private', '--source', '.', '--remote', 'origin', '--push', '--description', 'Hi',
    ])
  })

  it('reports a created repository whose push failed', async () => {
    const { host } = hostWith((command, args) => {
      if (command === 'gh') return fail('failed to run git: remote rejected')
      if (args[0] === 'remote' && args[1] === 'get-url') return ok('git@github.com:ada/app.git\n')
      if (args[0] === 'remote') return ok('origin\n')
      return ok('abc')
    })
    const result = await ghCreateRepository(host, '/p', { name: 'app', description: '', visibility: 'public' })
    expect(result).toEqual({ url: 'https://github.com/ada/app', pushError: 'failed to run git: remote rejected' })
  })

  it('says so when the name is taken', async () => {
    const { host } = hostWith((command, args) =>
      command === 'gh' ? fail('GraphQL: Name already exists on this account (createRepository)') : args[0] === 'remote' ? ok('') : ok('abc'),
    )
    await expect(ghCreateRepository(host, '/p', { name: 'app', description: '', visibility: 'public' })).rejects.toBeInstanceOf(NameTakenError)
  })
})

describe('ghIdentity', () => {
  it('uses the account name and the private noreply address', async () => {
    const { ghIdentity } = await import('../repo-setup')
    const { host } = hostWith(() => ok('583231\toctocat\tThe Octocat\n'))
    expect(await ghIdentity(host, '/p')).toEqual({ name: 'The Octocat', email: '583231+octocat@users.noreply.github.com' })
  })

  it('falls back to the login without a name, and to nothing without gh', async () => {
    const { ghIdentity } = await import('../repo-setup')
    expect(await ghIdentity(hostWith(() => ok('1\tada\t\n')).host, '/p')).toEqual({ name: 'ada', email: '1+ada@users.noreply.github.com' })
    expect(await ghIdentity(hostWith(() => new Error('program not found')).host, '/p')).toBeNull()
  })
})
