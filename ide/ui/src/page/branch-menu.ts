/* What a branch's page in the branch menu offers, worked out from where the
   folder stands: a local branch other than the folder's own gets the full
   set, the folder's own branch only what makes sense on it, a remote
   branch adds the pulls, and a tag gets what a tag takes. Pure, so the menu
   only draws it and the tests read it. */

export type BranchActionId =
  | 'checkout'
  | 'new-branch'
  | 'checkout-rebase'
  | 'compare'
  | 'diff-worktree'
  | 'rebase-onto'
  | 'merge-into'
  | 'new-worktree'
  | 'update'
  | 'push'
  | 'pull-rebase'
  | 'pull-merge'
  | 'tracked'
  | 'rename'
  | 'delete'

export interface BranchAction {
  id: BranchActionId
  label: string
  /** Actions sit in groups, parted by a separator. */
  group: number
  /** Why it can't run now; absent when it can. */
  disabled?: string
  danger?: boolean
}

export interface BranchRef {
  /** `feat/x`, or `origin/feat/x` for a remote branch, or a tag's name. */
  name: string
  remote: boolean
  tag?: boolean
}

/** What the branch menu asks of the Git window, by full ref names: the
    commits `ref` has that `against` lacks (Compare with), or what the
    working tree has different from `ref` (Show Diff with Working Tree). */
export type GitWindowRequest = { kind: 'compare'; ref: string; against: string } | { kind: 'diff'; ref: string }

/** The ref's full name, as the Git window's log knows it. */
export function fullRef(ref: BranchRef): string {
  return `refs/${ref.tag ? 'tags' : ref.remote ? 'remotes' : 'heads'}/${ref.name}`
}

export interface BranchContext {
  /** The folder's branch; null on a detached HEAD. */
  current: string | null
  defaultBranch: string | null
  /** The remote-tracking branch a local branch follows. */
  upstream: string | null
  /** The folder of another worktree that has this branch checked out. */
  checkedOutIn: string | null
  /** The folder of the worktree this branch is checked out in, when it can
      be removed (any but the main one, this folder's own included):
      Delete removes that worktree. */
  worktree: string | null
  /** The menu can open the Git window (the IDE's, not the chat's). */
  canDiff: boolean
  /** Where a tag is pushed; null with no remote. */
  remote: string | null
}

export function branchActions(ref: BranchRef, ctx: BranchContext): BranchAction[] {
  const { name } = ref
  const { current } = ctx
  const own = !ref.remote && name === current
  const detached = current === null ? 'the folder is on a detached HEAD' : undefined
  const actions: BranchAction[] = []
  const add = (id: BranchActionId, label: string, group: number, extra: Partial<BranchAction> = {}) =>
    actions.push({ id, label, group, ...extra })

  if (ref.tag) {
    add('checkout', 'Checkout', 0)
    add('new-branch', `New Branch from '${name}'…`, 0)
    if (ctx.canDiff) {
      add('compare', `Compare with '${current ?? 'HEAD'}'`, 1)
      add('diff-worktree', 'Show Diff with Working Tree', 1)
    }
    add('merge-into', `Merge '${name}' into '${current ?? 'HEAD'}'`, 2, { disabled: detached })
    add('new-worktree', `New Worktree from '${name}'…`, 3)
    add('push', ctx.remote === null ? 'Push' : `Push to '${ctx.remote}'`, 4, {
      disabled: ctx.remote === null ? 'the repository has no remote' : undefined,
    })
    add('delete', 'Delete', 5, { danger: true })
    return actions
  }

  if (!own) {
    add('checkout', 'Checkout', 0, {
      disabled:
        ctx.checkedOutIn === null ? undefined : `checked out in ${ctx.checkedOutIn}: pick its row to switch there`,
    })
  }
  add('new-branch', `New Branch from '${name}'…`, 0)
  if (!own) {
    add('checkout-rebase', `Checkout and Rebase onto '${current ?? 'HEAD'}'`, 0, {
      disabled: detached ?? (ctx.checkedOutIn === null ? undefined : `checked out in ${ctx.checkedOutIn}`),
    })
  }
  if (ctx.canDiff) {
    if (!own) add('compare', `Compare with '${current ?? 'HEAD'}'`, 1)
    add('diff-worktree', 'Show Diff with Working Tree', 1)
  }
  if (!own) {
    add('rebase-onto', `Rebase '${current ?? 'HEAD'}' onto '${name}'`, 2, { disabled: detached })
    add('merge-into', `Merge '${name}' into '${current ?? 'HEAD'}'`, 2, { disabled: detached })
  }
  add('new-worktree', `New Worktree from '${name}'…`, 3)

  if (ref.remote) {
    add('pull-rebase', `Pull into '${current ?? 'HEAD'}' Using Rebase`, 4, { disabled: detached })
    add('pull-merge', `Pull into '${current ?? 'HEAD'}' Using Merge`, 4, { disabled: detached })
    const defaultUpstream = ctx.defaultBranch !== null && name.endsWith(`/${ctx.defaultBranch}`)
    add('delete', 'Delete', 5, {
      danger: true,
      disabled: defaultUpstream ? `${name} is the default branch on its remote` : undefined,
    })
    return actions
  }

  add('update', 'Update', 4, { disabled: ctx.upstream === null ? `${name} tracks no remote branch` : undefined })
  add('push', 'Push…', 4)
  if (ctx.upstream !== null) add('tracked', `Tracked Branch '${ctx.upstream}'`, 5)
  add('rename', 'Rename…', 6, {
    disabled: name === ctx.defaultBranch ? `${name} is the default branch; it keeps its name` : undefined,
  })
  if (ctx.worktree !== null) {
    add('delete', `Delete Worktree '${ctx.worktree}'…`, 6, { danger: true })
  } else if (!own) {
    add('delete', 'Delete', 6, {
      danger: true,
      disabled:
        name === ctx.defaultBranch
          ? `${name} is the default branch`
          : ctx.checkedOutIn === null
            ? undefined
            : `checked out in ${ctx.checkedOutIn}`,
    })
  }
  return actions
}
