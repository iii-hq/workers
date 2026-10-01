import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import { type GitState, gitChanges } from '../git'
import { useSourceControl } from '../use-source-control'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))

// A repository whose status and HEAD diff the test sets (a null diff times
// out); every git run is recorded by its first two arguments.
function repository(status: string, diff: string | null) {
  const repo = { status, diff, runs: [] as string[] }
  const trigger = async (_fn: string, payload: { args: string[] }) => {
    // Every read runs with --no-optional-locks first (git.ts READ_ONLY).
    const [verb, flag] = payload.args[0] === '--no-optional-locks' ? payload.args.slice(1) : payload.args
    repo.runs.push(`${verb} ${flag}`)
    const stdout =
      verb === 'status'
        ? repo.status
        : verb === 'diff'
          ? (repo.diff ?? '')
          : flag === '--is-inside-work-tree'
            ? 'true\n'
            : `${'a'.repeat(40)}\n`
    const timedOut = verb === 'diff' && repo.diff === null
    return { exit_code: 0, stdout, stderr: '', timed_out: timedOut, stdout_truncated: false, stderr_truncated: false }
  }
  return { repo, host: { iii: { trigger } } as unknown as Host }
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('the commit panel', () => {
  it("derives from the page's status, again on every page refresh", async () => {
    const { repo, host } = repository('## main...origin/main [different]\0 M a.ts\0', 'M\0a.ts\0')
    const first = await gitChanges(host, '/repo')
    repo.runs = []
    const refresh = vi.fn(async () => null)
    const panel = mount(
      ({ git, epoch }: { git: GitState | null; epoch: number }) =>
        useSourceControl(host, '/repo', epoch, true, () => {}, { git, refresh }),
      { git: first, epoch: 0 },
    )
    await settle()
    // The panel itself only runs what the status cannot tell.
    expect(repo.runs).toEqual(['rev-parse --verify', 'diff --no-ext-diff'])
    expect(panel.result).toMatchObject({ phase: 'ready', branch: 'main' })
    expect(panel.result.changes.map((entry) => entry.path)).toEqual(['a.ts'])

    // A page refresh that kept its object derives again: the panel's own
    // read may have failed, and is retried rather than left on its error.
    repo.diff = null
    panel.rerender({ git: first, epoch: 1 })
    await settle()
    expect(panel.result).toMatchObject({ phase: 'error', error: 'git diff uncommitted timed out' })
    repo.diff = 'M\0a.ts\0'
    panel.rerender({ git: first, epoch: 2 })
    await settle()
    expect(panel.result).toMatchObject({ phase: 'ready', error: null })

    // A changed status derives too.
    repo.status = '## main\0 M a.ts\0?? b.ts\0'
    const second = await gitChanges(host, '/repo', first)
    repo.runs = []
    panel.rerender({ git: second, epoch: 2 })
    await settle()
    expect(repo.runs).toEqual(['rev-parse --verify', 'diff --no-ext-diff'])
    expect(panel.result.unversioned.map((entry) => entry.path)).toEqual(['b.ts'])

    // A status that fails to read shows the error and keeps the branch, so
    // the commit box keeps its message.
    panel.rerender({ git: { kind: 'error', message: 'git status timed out' }, epoch: 3 })
    await settle()
    expect(panel.result).toMatchObject({ phase: 'error', error: 'git status timed out', branch: 'main' })

    // Refresh asks the page to read its status again, not quietly: the
    // epoch it bumps derives even when the status came back the same.
    panel.rerender({ git: second, epoch: 4 })
    await settle()
    repo.runs = []
    panel.result.reload()
    expect(refresh).toHaveBeenLastCalledWith()
    expect(panel.result.refreshing).toBe(true)
    panel.rerender({ git: second, epoch: 5 })
    await settle()
    expect(repo.runs).toEqual(['rev-parse --verify', 'diff --no-ext-diff'])
    expect(panel.result).toMatchObject({ phase: 'ready', refreshing: false })
    panel.unmount()
  })

  it("asks the page to read again on opening, quietly, unless the page's first read is in flight", async () => {
    const { host } = repository('## main\0 M a.ts\0', 'M\0a.ts\0')
    const status = await gitChanges(host, '/repo')
    const refresh = vi.fn(async () => null)
    const panel = mount(
      ({ git, active }: { git: GitState | null; active: boolean }) =>
        useSourceControl(host, '/repo', 0, active, () => {}, { git, refresh }),
      { git: null, active: true },
    )
    await settle()
    // No status yet: the root's first read is already on its way.
    panel.rerender({ git: status, active: true })
    await settle()
    expect(refresh).not.toHaveBeenCalled()
    expect(panel.result.phase).toBe('ready')

    // Quiet, so the epoch stays put: the Stash and History tabs just loaded
    // on mounting with the view, and would read again on a bump.
    panel.rerender({ git: status, active: false })
    panel.rerender({ git: status, active: true })
    await settle()
    expect(refresh).toHaveBeenCalledTimes(1)
    expect(refresh).toHaveBeenCalledWith({ quiet: true })
    panel.unmount()
  })
})
