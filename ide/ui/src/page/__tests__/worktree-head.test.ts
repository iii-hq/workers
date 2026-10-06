import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseHead, readHead, useHead } from '../worktree-head'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))
vi.mock('../use-worktree-ops', () => ({ useWorktreeEpoch: () => 0 }))

const ok = (stdout: string) => ({ exit_code: 0, stdout })
// What the path probe prints in a main checkout and in a linked worktree.
const MAIN = ok('/r\n/r/.git\n/r/.git\n')
const LINKED = ok('/r.feat-x\n/r/.git/worktrees/r.feat-x\n/r/.git\n')
const CLEAN = ok('')

describe('parseHead', () => {
  it('names the branch from symbolic-ref, a tag of the same name notwithstanding', () => {
    expect(parseHead(ok('refs/heads/main\n'), MAIN, CLEAN)).toEqual({
      branch: 'main',
      worktree: '/r',
      linked: false,
      dirty: false,
    })
    expect(parseHead(ok('refs/heads/feat/x\n'), LINKED, ok('?? scratch.txt\n'))).toEqual({
      branch: 'feat/x',
      worktree: '/r.feat-x',
      linked: true,
      dirty: true,
    })
  })

  it('reads a detached HEAD, and old git echoing --path-format', () => {
    expect(parseHead({ exit_code: 1, stdout: '' }, MAIN, CLEAN)?.branch).toBeNull()
    // git < 2.31 in a main checkout.
    expect(parseHead(ok('refs/heads/main\n'), ok('/r\n/r/.git\n--path-format=absolute\n.git\n'), CLEAN)?.linked).toBe(
      false,
    )
  })

  it('shows nothing outside a work tree or when a probe dies', () => {
    const fatal = { exit_code: 128, stdout: '' }
    expect(parseHead(fatal, fatal, fatal)).toBeNull()
    // Inside `.git` or a bare repository: a branch, but no work tree.
    expect(parseHead(ok('refs/heads/main\n'), fatal, CLEAN)).toBeNull()
    expect(parseHead(null, MAIN, CLEAN)).toBeNull()
    // A status that fails only leaves the dirty mark off.
    expect(parseHead(ok('refs/heads/main\n'), MAIN, null)?.dirty).toBe(false)
  })
})

describe('readHead', () => {
  it('shares one read of a folder until it ages or a worktree operation ends', async () => {
    const calls: string[] = []
    const host = {
      iii: {
        trigger: async (_fn: string, { args, cwd }: { args: string[]; cwd: string }) => {
          calls.push(`${cwd} ${args[0]}`)
          return args[0] === 'symbolic-ref' ? ok('refs/heads/main\n') : args[0] === 'rev-parse' ? MAIN : CLEAN
        },
      },
    } as unknown as Host

    // The IDE's header and the chat's composer on the same folder: one read.
    const first = readHead(host, '/shared', 0, 1000)
    expect(readHead(host, '/shared', 0, 1000)).toBe(first)
    expect(calls).toHaveLength(3)
    expect(await first).toMatchObject({ branch: 'main', dirty: false })
    // Another folder, a read too old, or an operation that ended since: read again.
    readHead(host, '/other', 0, 1000)
    expect(calls).toHaveLength(6)
    expect(readHead(host, '/shared', 0, 0)).not.toBe(first)
    expect(calls).toHaveLength(9)
    readHead(host, '/shared', 1, 1000)
    expect(calls).toHaveLength(12)
  })
})

describe('useHead', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('updates two chips on one folder on the same focus', async () => {
    let branch = 'main'
    let calls = 0
    const host = {
      iii: {
        trigger: async (_fn: string, { args }: { args: string[] }) => {
          calls += 1
          return args[0] === 'symbolic-ref' ? ok(`refs/heads/${branch}\n`) : args[0] === 'rev-parse' ? MAIN : CLEAN
        },
      },
    } as unknown as Host
    vi.stubGlobal('window', new EventTarget())
    vi.stubGlobal('document', Object.assign(new EventTarget(), { visibilityState: 'visible' }))
    let now = 0
    vi.spyOn(Date, 'now').mockImplementation(() => now)
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

    // The IDE's header and the chat's composer on the same folder.
    const header = mount(() => useHead(host, '/focused', 'turn'), null)
    const composer = mount(() => useHead(host, '/focused', 'turn'), null)
    await settle()
    expect(calls).toBe(3)
    expect([header.result?.branch, composer.result?.branch]).toEqual(['main', 'main'])

    // A `git switch` in the terminal, long after, then focus moves: one
    // read, and both chips show it at once.
    branch = 'feat/x'
    now = 10_000
    document.dispatchEvent(new Event('focusin'))
    await settle()
    expect(calls).toBe(6)
    expect([header.result?.branch, composer.result?.branch]).toEqual(['feat/x', 'feat/x'])
    header.unmount()
    composer.unmount()
  })
})
