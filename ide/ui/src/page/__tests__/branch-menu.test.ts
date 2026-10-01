import { describe, expect, it, vi } from 'vitest'
import {
  CheckoutBlocked,
  checkoutBranch,
  checkoutRemoteBranch,
  conflictNote,
  overwrittenFiles,
  parseRecentBranches,
  parseRemoteBranches,
  RemoteDiverged,
  splitRemote,
} from '../branch-actions'
import { type BranchContext, branchActions } from '../branch-menu'
import { pushDescription } from '../WorktreeSwitcher'
import { parseBranches } from '../worktrees'

// The shared components only exist inside the console; these tests read pure helpers.
vi.mock('@iii-dev/console-ui', () => ({}))

const ctx = (over: Partial<BranchContext> = {}): BranchContext => ({
  current: 'main',
  defaultBranch: 'main',
  upstream: 'origin/feat/x',
  checkedOutIn: null,
  worktree: null,
  canDiff: true,
  remote: 'origin',
  ...over,
})

const ids = (actions: { id: string }[]) => actions.map((action) => action.id)

describe('branchActions', () => {
  it('offers another local branch the full set, without a commit action', () => {
    const actions = branchActions({ name: 'feat/x', remote: false }, ctx())
    expect(ids(actions)).toEqual([
      'checkout',
      'new-branch',
      'checkout-rebase',
      'compare',
      'diff-worktree',
      'rebase-onto',
      'merge-into',
      'new-worktree',
      'update',
      'push',
      'tracked',
      'rename',
      'delete',
    ])
    expect(actions.find((action) => action.id === 'checkout-rebase')?.label).toBe("Checkout and Rebase onto 'main'")
    expect(actions.find((action) => action.id === 'merge-into')?.label).toBe("Merge 'feat/x' into 'main'")
    expect(actions.find((action) => action.id === 'tracked')?.label).toBe("Tracked Branch 'origin/feat/x'")
    expect(actions.every((action) => action.disabled === undefined)).toBe(true)
  })

  it('leaves the folder its own branch without checkout, compare, rebase, merge or delete', () => {
    const actions = branchActions({ name: 'main', remote: false }, ctx({ upstream: 'origin/main' }))
    expect(ids(actions)).toEqual(['new-branch', 'diff-worktree', 'new-worktree', 'update', 'push', 'tracked', 'rename'])
    expect(actions.find((action) => action.id === 'rename')?.disabled).toMatch(/default branch/)
  })

  it('says why: another worktree has it, no upstream, a detached HEAD', () => {
    const elsewhere = branchActions(
      { name: 'feat/x', remote: false },
      ctx({ checkedOutIn: 'repo.feat-x', upstream: null }),
    )
    expect(elsewhere.find((action) => action.id === 'checkout')?.disabled).toMatch(/repo\.feat-x/)
    expect(elsewhere.find((action) => action.id === 'delete')?.disabled).toMatch(/repo\.feat-x/)
    expect(elsewhere.find((action) => action.id === 'update')?.disabled).toMatch(/tracks no remote/)
    expect(ids(elsewhere)).not.toContain('tracked')
    const detached = branchActions({ name: 'feat/x', remote: false }, ctx({ current: null }))
    expect(detached.find((action) => action.id === 'merge-into')?.disabled).toMatch(/detached/)
    expect(detached.find((action) => action.id === 'merge-into')?.label).toBe("Merge 'feat/x' into 'HEAD'")
  })

  it('gives a remote branch the pulls, and keeps the default one from deletion', () => {
    const remote = branchActions({ name: 'origin/main', remote: true }, ctx({ current: 'feat/x', upstream: null }))
    expect(ids(remote)).toEqual([
      'checkout',
      'new-branch',
      'checkout-rebase',
      'compare',
      'diff-worktree',
      'rebase-onto',
      'merge-into',
      'new-worktree',
      'pull-rebase',
      'pull-merge',
      'delete',
    ])
    expect(remote.find((action) => action.id === 'pull-rebase')?.label).toBe("Pull into 'feat/x' Using Rebase")
    expect(remote.find((action) => action.id === 'delete')?.disabled).toMatch(/default branch/)
  })

  it('deletes the worktree a branch is checked out in, this folder its own included', () => {
    const other = branchActions(
      { name: 'feat/x', remote: false },
      ctx({ checkedOutIn: 'repo.feat-x', worktree: 'repo.feat-x' }),
    )
    const del = other.find((action) => action.id === 'delete')
    expect(del?.label).toBe("Delete Worktree 'repo.feat-x'…")
    expect(del?.disabled).toBeUndefined()
    const own = branchActions({ name: 'feat/x', remote: false }, ctx({ current: 'feat/x', worktree: 'repo.feat-x' }))
    expect(own.find((action) => action.id === 'delete')?.label).toBe("Delete Worktree 'repo.feat-x'…")
  })

  it('gives a tag what a tag takes, its push naming the remote', () => {
    const tag = branchActions({ name: 'v1.2.0', remote: false, tag: true }, ctx({ current: 'feat/x' }))
    expect(ids(tag)).toEqual([
      'checkout',
      'new-branch',
      'compare',
      'diff-worktree',
      'merge-into',
      'new-worktree',
      'push',
      'delete',
    ])
    expect(tag.find((action) => action.id === 'push')?.label).toBe("Push to 'origin'")
    const lone = branchActions({ name: 'v1.2.0', remote: false, tag: true }, ctx({ remote: null }))
    expect(lone.find((action) => action.id === 'push')?.disabled).toMatch(/no remote/)
  })

  it('sets New Worktree apart from Rebase and Merge, as its own group', () => {
    const actions = branchActions({ name: 'origin/main', remote: true }, ctx({ current: 'feat/x' }))
    const group = (id: string) => actions.find((action) => action.id === id)?.group
    expect(group('merge-into')).not.toBe(group('new-worktree'))
    expect(group('new-worktree')).not.toBe(group('pull-rebase'))
  })

  it('leaves the diffs out where the menu cannot open them (the chat chip)', () => {
    expect(ids(branchActions({ name: 'feat/x', remote: false }, ctx({ canDiff: false })))).not.toContain('compare')
  })
})

describe('branch lists', () => {
  it('reads recent checkouts from the reflog, newest first, only live branches, never the current one', () => {
    const reflog = [
      'checkout: moving from feat/a to main',
      'commit: wip',
      'checkout: moving from main to feat/a',
      'checkout: moving from gone to feat/b',
      'checkout: moving from feat/b to feat/c',
    ].join('\n')
    expect(parseRecentBranches(reflog, ['main', 'feat/a', 'feat/b', 'feat/c'], 'main')).toEqual([
      'feat/a',
      'feat/b',
      'feat/c',
    ])
    expect(parseRecentBranches(reflog, ['main', 'feat/a', 'feat/b', 'feat/c'], 'main', 2)).toEqual(['feat/a', 'feat/b'])
  })

  it('lists remote branches without the remote HEAD', () => {
    expect(parseRemoteBranches('origin/main\norigin/HEAD\norigin\nupstream/feat/x\n')).toEqual([
      'origin/main',
      'upstream/feat/x',
    ])
    expect(splitRemote('origin/feat/x')).toEqual({ remote: 'origin', branch: 'feat/x' })
  })

  it('reads each local branch with its upstream and its counts', () => {
    expect(parseBranches('refs/heads/main\torigin/main\t0 0\nrefs/heads/feat/x\t\t2 5\nrefs/heads/solo\t\n')).toEqual([
      { name: 'main', upstream: 'origin/main', ahead: 0, behind: 0 },
      { name: 'feat/x', ahead: 2, behind: 5 },
      { name: 'solo' },
    ])
  })
})

function reply(overrides: Partial<{ exit_code: number; stdout: string; stderr: string }> = {}) {
  return {
    exit_code: 0,
    stdout: '',
    stderr: '',
    timed_out: false,
    stdout_truncated: false,
    stderr_truncated: false,
    ...overrides,
  }
}

function hostAnswering(answer: (args: string[]) => ReturnType<typeof reply>) {
  const calls: string[][] = []
  const trigger = vi.fn(async (_fn: string, payload: { args: string[] }) => {
    calls.push(payload.args)
    return answer(payload.args)
  })
  return { host: { iii: { trigger } } as unknown as Parameters<typeof checkoutBranch>[0], calls }
}

describe('branch verbs', () => {
  it('turns a stopped merge or rebase into what to do next', () => {
    expect(conflictNote('rebase', 'CONFLICT (content): Merge conflict in a.ts\nerror: could not apply 1a2b3c')).toMatch(
      /rebase --continue/,
    )
    expect(conflictNote('merge', 'Automatic merge failed; fix conflicts and then commit the result.')).toMatch(
      /merge --abort/,
    )
    expect(conflictNote('merge', 'fatal: refusing to merge unrelated histories')).toBeNull()
  })

  it('checks a remote branch out through a new tracking branch, or resets the local one with nothing of its own', async () => {
    const made = hostAnswering(() => reply())
    await expect(checkoutRemoteBranch(made.host, '/r', 'origin/feat/x', ['main'])).resolves.toMatch(
      /tracking origin\/feat\/x/,
    )
    expect(made.calls).toEqual([['switch', '--quiet', '--track', 'origin/feat/x']])
    const behind = hostAnswering((args) => (args[0] === 'rev-list' ? reply({ stdout: '0\t3\n' }) : reply()))
    await expect(checkoutRemoteBranch(behind.host, '/r', 'origin/feat/x', ['main', 'feat/x'])).resolves.toMatch(
      /reset to origin\/feat\/x/,
    )
    expect(behind.calls.at(-1)).toEqual(['switch', '--quiet', '-C', 'feat/x', '--track', 'origin/feat/x'])
  })

  it('stops on a local branch with commits of its own, until told to drop or rebase them', async () => {
    const ahead = hostAnswering((args) => (args[0] === 'rev-list' ? reply({ stdout: '2\t3\n' }) : reply()))
    await expect(checkoutRemoteBranch(ahead.host, '/r', 'origin/feat/x', ['feat/x'])).rejects.toBeInstanceOf(
      RemoteDiverged,
    )
    const dropped = hostAnswering(() => reply())
    await checkoutRemoteBranch(dropped.host, '/r', 'origin/feat/x', ['feat/x'], 'plain', 'drop')
    expect(dropped.calls).toEqual([['switch', '--quiet', '-C', 'feat/x', '--track', 'origin/feat/x']])
    const rebased = hostAnswering(() => reply())
    await checkoutRemoteBranch(rebased.host, '/r', 'origin/feat/x', ['feat/x'], 'plain', 'rebase')
    expect(rebased.calls).toEqual([
      ['rebase', '--autostash', 'origin/feat/x', 'feat/x'],
      ['branch', '--set-upstream-to=origin/feat/x', 'feat/x'],
    ])
  })

  it('names the files a checkout would overwrite, and forces or stashes across it when told', async () => {
    const output = [
      'error: Your local changes to the following files would be overwritten by checkout:',
      '\ta.ts',
      'Please commit your changes or stash them before you switch branches.',
      'error: The following untracked working tree files would be overwritten by checkout:',
      '\tnew.ts',
      'Please move or remove them before you switch branches.',
      'Aborting',
    ].join('\n')
    expect(overwrittenFiles(output)).toEqual(['a.ts', 'new.ts'])
    expect(overwrittenFiles("error: pathspec 'nope' did not match")).toBeNull()
    const blocked = hostAnswering(() => reply({ exit_code: 1, stderr: output }))
    await expect(checkoutBranch(blocked.host, '/r', 'feat/x')).rejects.toBeInstanceOf(CheckoutBlocked)
    const forced = hostAnswering(() => reply())
    await checkoutBranch(forced.host, '/r', 'feat/x', 'force')
    expect(forced.calls).toEqual([['switch', '--quiet', '--force', 'feat/x']])
    // Smart: stashed (the stash tip moves), switched, brought back.
    let tip = 'old'
    const smart = hostAnswering((args) => {
      if (args.includes('refs/stash')) return reply({ stdout: `${tip}\n` })
      if (args[0] === 'stash' && args[1] === 'push') tip = 'new'
      return reply()
    })
    await checkoutBranch(smart.host, '/r', 'feat/x', 'smart')
    const verbs = smart.calls.filter((args) => !args.includes('refs/stash')).map((args) => args.slice(0, 2).join(' '))
    expect(verbs).toEqual(['stash push', 'switch --quiet', 'stash pop'])
  })

  it('describes a push before it runs', () => {
    const target = { remote: 'origin', ref: 'refs/heads/feat/x', name: 'origin/feat/x', setUpstream: false, ahead: 3 }
    expect(pushDescription({ branch: 'feat/x', target })).toBe('3 commits to push.')
    expect(pushDescription({ branch: 'feat/x', target: { ...target, setUpstream: true, ahead: null } })).toMatch(
      /becomes it/,
    )
    expect(pushDescription(null)).toBeUndefined()
  })
})
