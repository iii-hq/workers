import type { Host } from '@iii-dev/console-ui'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { publishWorktree, WORKTREE_COALESCE_MS } from '../git-watch'
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

// An engine client that answers the three gits and keeps the bindings the
// chips make, so a test can fire what the ide worker would.
function engine(answer: (args: string[]) => { exit_code: number; stdout: string }) {
  const calls: string[] = []
  const handlers = new Map<string, (payload: unknown) => void>()
  const triggers: Array<{ type: string; function_id: string; config: Record<string, unknown> }> = []
  const host = {
    iii: {
      browserId: 'tab',
      trigger: async (_fn: string, { args }: { args: string[] }) => {
        calls.push(args[0] === '--no-optional-locks' ? 'status' : args[0])
        return answer(args)
      },
      on: (functionId: string, handler: (payload: unknown) => void) => {
        handlers.set(functionId, handler)
        return () => handlers.delete(functionId)
      },
      registerTrigger: (input: (typeof triggers)[number]) => {
        triggers.push(input)
        return () => triggers.splice(triggers.indexOf(input), 1)
      },
    },
  } as unknown as Host
  const bound = (type: string) => triggers.filter((t) => t.type === type)
  const fire = (type: string, payload: unknown) => {
    for (const t of bound(type)) handlers.get(t.function_id.replace(/::tab$/, ''))?.(payload)
  }
  return { host, calls, bound, fire }
}

describe('useHead', () => {
  afterEach(() => {
    vi.useRealTimers()
  })
  const settle = () => vi.advanceTimersByTimeAsync(0)

  it('reads both chips on one folder again when the worker reports a switch, and never on focus', async () => {
    vi.useFakeTimers()
    let branch = 'main'
    const { host, calls, bound, fire } = engine((args) =>
      args[0] === 'symbolic-ref' ? ok(`refs/heads/${branch}\n`) : args[0] === 'rev-parse' ? MAIN : CLEAN,
    )
    // The IDE's header and the chat's composer on the same folder.
    const header = mount(() => useHead(host, '/switched', 'turn'), null)
    const composer = mount(() => useHead(host, '/switched', 'turn'), null)
    await settle()
    expect(calls).toHaveLength(3)
    expect([header.result?.branch, composer.result?.branch]).toEqual(['main', 'main'])
    // One repository binding for both chips, one worktree binding.
    expect(bound('shell::git-changed').map((t) => t.config)).toEqual([{ path: '/switched' }])
    expect(bound('shell::changed').map((t) => t.config)).toEqual([{ path: '/switched' }])

    // Time passing reads nothing: there is no timer and no focus listener.
    await vi.advanceTimersByTimeAsync(60_000)
    expect(calls).toHaveLength(3)

    // A `git switch` in the terminal: one read, and both chips show it.
    branch = 'feat/x'
    fire('shell::git-changed', { path: '/switched', changes: ['head', 'refs'] })
    await settle()
    expect(calls).toHaveLength(6)
    expect([header.result?.branch, composer.result?.branch]).toEqual(['feat/x', 'feat/x'])
    header.unmount()
    composer.unmount()
    expect(bound('shell::git-changed')).toEqual([])
    expect(bound('shell::changed')).toEqual([])
  })

  it('reads only the dirty mark when files or the index change', async () => {
    vi.useFakeTimers()
    let dirty = false
    const { host, calls, fire } = engine((args) =>
      args[0] === 'symbolic-ref'
        ? ok('refs/heads/main\n')
        : args[0] === 'rev-parse'
          ? MAIN
          : ok(dirty ? ' M a.txt\n' : ''),
    )
    const chip = mount(() => useHead(host, '/edited', 'turn'), null)
    await settle()
    expect(chip.result?.dirty).toBe(false)
    calls.length = 0

    // A burst of saves is one status read, once the burst settles.
    dirty = true
    for (const path of ['a.txt', 'b.txt', 'c.txt']) fire('shell::changed', { path, kind: 'modified', root: '/edited' })
    await settle()
    expect(calls).toEqual([])
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(calls).toEqual(['status'])
    expect(chip.result).toMatchObject({ branch: 'main', dirty: true })

    // A commit in a terminal moves only the index here: status again.
    dirty = false
    fire('shell::git-changed', { path: '/edited', changes: ['index'] })
    await settle()
    expect(calls).toEqual(['status', 'status'])
    expect(chip.result?.dirty).toBe(false)
    chip.unmount()
  })

  it('leaves a folder in no repository alone until one appears there', async () => {
    vi.useFakeTimers()
    let repo = false
    const fatal = { exit_code: 128, stdout: '' }
    const { host, calls, bound, fire } = engine((args) =>
      !repo ? fatal : args[0] === 'symbolic-ref' ? ok('refs/heads/main\n') : args[0] === 'rev-parse' ? MAIN : CLEAN,
    )
    let key = 'idle'
    const chip = mount(() => useHead(host, '/plain', key), null)
    await settle()
    expect(chip.result).toBeNull()
    expect(calls).toHaveLength(3)
    // No status reads for files in no repository, and a turn ending reads nothing.
    expect(bound('shell::changed')).toEqual([])
    key = 'streaming'
    chip.rerender(null)
    await settle()
    expect(calls).toHaveLength(3)

    // `git init`: the worker watching the folder for a .git says so.
    repo = true
    fire('shell::git-changed', { path: '/plain', changes: ['repository'] })
    await settle()
    expect(calls).toHaveLength(6)
    expect(chip.result?.branch).toBe('main')
    expect(bound('shell::changed').map((t) => t.config)).toEqual([{ path: '/plain' }])
    chip.unmount()
  })

  it('takes the IDE page’s file events instead of binding a watch of its own', async () => {
    vi.useFakeTimers()
    const { host, calls, bound } = engine((args) =>
      args[0] === 'symbolic-ref' ? ok('refs/heads/main\n') : args[0] === 'rev-parse' ? MAIN : CLEAN,
    )
    const page = publishWorktree(host, '/ide-root')
    const chip = mount(() => useHead(host, '/ide-root', 'turn'), null)
    await settle()
    expect(bound('shell::changed')).toEqual([])
    calls.length = 0
    page.note()
    await vi.advanceTimersByTimeAsync(WORKTREE_COALESCE_MS)
    expect(calls).toEqual(['status'])
    // The page goes: the chip binds its own watch.
    page.off()
    expect(bound('shell::changed').map((t) => t.config)).toEqual([{ path: '/ide-root' }])
    chip.unmount()
    expect(bound('shell::changed')).toEqual([])
  })
})
