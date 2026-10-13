/* One branch's (or tag's) actions in the branch menu (`branch-menu.ts`
   decides which), in a menu beside the list or, with no room beside it, in
   its place, with the name forms some of them ask for. "Compare with" and
   "Show Diff with Working Tree" open in the Git window. Beside the list,
   Tracked Branch opens the tracked branch's actions in a menu beside this
   one. Escape or ← steps back a level. */

import { DropdownMenuSeparator, IconButton, List, uiClasses } from '@iii-dev/console-ui'
import { ArrowLeft, ChevronRight } from 'lucide-react'
import { Fragment, type KeyboardEvent, useEffect, useRef, useState } from 'react'
import {
  type BranchActionId,
  type BranchContext,
  type BranchRef,
  branchActions,
  fullRef,
  type GitWindowRequest,
} from './branch-menu'
import { type Beside, besideOf, Flyout, useHoverIntent } from './flyout'
import type { WorktreeOps } from './use-worktree-ops'
import { BranchNameForm } from './WorktreeForms'
import type { Worktree } from './worktrees'

type NameForm = 'new-branch' | 'new-worktree' | 'rename'

export interface BranchPageProps {
  branch: BranchRef
  ctx: BranchContext
  /** The removable worktree the branch is checked out in: Delete removes it. */
  worktree: Worktree | null
  ops: WorktreeOps
  /** Beside the list: no header, as a menu has none. */
  flyout?: boolean
  /** A form is open: hovering another branch must not take it away. */
  onSettle?: (settled: boolean) => void
  busy: boolean
  onBack: () => void
  /** Opens the page of the remote branch this one tracks in this one's
      place: in place of the list, or beside it with no room beside this. */
  onTracked: (upstream: string) => void
  /** Closes the menu: after a confirmation that is a dialog of its own, a move or the Git window opened. */
  onClose: () => void
  /** Opens the Git window on a comparison; without it (the chat's chip) there is none. */
  onShowInGit?: (request: GitWindowRequest) => void
}

export function BranchPage({
  branch,
  ctx,
  worktree,
  ops,
  flyout = false,
  onSettle,
  busy,
  onBack,
  onTracked,
  onClose,
  onShowInGit,
}: BranchPageProps) {
  const [form, setForm] = useState<NameForm | null>(null)
  const { name } = branch
  const current = ctx.current ?? 'HEAD'
  const actions = branchActions(branch, ctx)
  // The tracked branch's actions, in a menu beside this one.
  const [tracked, setTracked] = useState<{ at: Beside; focus: boolean } | null>(null)
  const [trackedSettled, setTrackedSettled] = useState(false)
  const trackedRow = useRef<HTMLElement | null>(null)
  const hover = useHoverIntent()
  const settled = form !== null || trackedSettled
  // Gone (a filter typed, Back), the page leaves the menu unsettled.
  useEffect(() => {
    onSettle?.(settled)
    return () => onSettle?.(false)
  }, [onSettle, settled])

  const openTracked = (row: HTMLElement, focus: boolean) => {
    const menu = row.closest<HTMLElement>('.shui-wt-flyout')
    const at = menu === null ? null : besideOf(menu, row)
    if (at === null) {
      if (ctx.upstream !== null) onTracked(ctx.upstream)
      return
    }
    trackedRow.current = row
    setTracked((open) => (open !== null && !focus ? open : { at, focus }))
  }
  const closeTracked = (refocus: boolean) => {
    hover.cancel()
    setTracked(null)
    setTrackedSettled(false)
    if (refocus) trackedRow.current?.querySelector<HTMLElement>('[data-list-item]')?.focus()
  }
  // Resting the pointer on Tracked Branch opens its menu; on another action, closes it.
  const actionsHover = () =>
    flyout
      ? hover.listProps('.shui-wt-action', (row) => {
          if (row === null || trackedSettled) return
          if (row.dataset.action === 'tracked') openTracked(row, false)
          else if (tracked !== null) closeTracked(false)
        })
      : {}

  const act = (id: BranchActionId) => {
    switch (id) {
      case 'checkout':
        if (branch.tag) ops.checkoutRevision(name)
        else if (branch.remote) ops.checkoutRemote(name)
        else ops.checkout(name)
        return onBack()
      case 'checkout-rebase':
        ops.checkoutAndRebase(name, current, branch.remote)
        return onBack()
      case 'rebase-onto':
        ops.rebaseCurrent(name)
        return onBack()
      case 'merge-into':
        ops.mergeIntoCurrent(name)
        return onBack()
      case 'pull-rebase':
      case 'pull-merge':
        ops.pullInto(name, id === 'pull-rebase')
        return onBack()
      case 'update':
        ops.updateBranch(name)
        return onBack()
      // In the Git window: the branch's commits the folder's branch lacks,
      // or what the working tree has different from it.
      case 'compare':
        onShowInGit?.({
          kind: 'compare',
          ref: fullRef(branch),
          against: ctx.current === null ? 'HEAD' : `refs/heads/${ctx.current}`,
        })
        return onClose()
      case 'diff-worktree':
        onShowInGit?.({ kind: 'diff', ref: fullRef(branch) })
        return onClose()
      case 'new-branch':
      case 'new-worktree':
      case 'rename':
        return setForm(id)
      case 'tracked':
        if (ctx.upstream !== null) onTracked(ctx.upstream)
        return
      case 'push':
        if (branch.tag) {
          if (ctx.remote !== null) ops.pushTag(name, ctx.remote)
          return onBack()
        }
        // The confirmation is a dialog of its own: the menu gives way.
        ops.askPush(name)
        return onClose()
      case 'delete':
        if (branch.tag) ops.askDeleteTag(name)
        else if (worktree !== null) ops.askRemove(worktree)
        else if (branch.remote) ops.askDeleteRemote(name)
        else ops.askDeleteBranch(name)
        return onClose()
    }
  }

  const back = () => {
    if (form !== null) setForm(null)
    else onBack()
  }
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // Escape and ← step back a level instead of closing the whole menu;
    // inside a field, ← moves the caret.
    const inField = (event.target as HTMLElement).tagName === 'INPUT'
    if (event.key === 'Escape' || (event.key === 'ArrowLeft' && !inField)) {
      event.preventDefault()
      event.stopPropagation()
      back()
    }
  }

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Escape and ← step back a level from anywhere on the page
    <div className="shui-wt-page" onKeyDown={onKeyDown}>
      {flyout ? null : (
        <div className="shui-wt-page-head">
          <IconButton label="Back to branches" variant="ghost" onClick={back}>
            <ArrowLeft size={16} aria-hidden />
          </IconButton>
          <span className="shui-wt-page-title" title={name}>
            {name}
          </span>
          {ctx.upstream !== null && !branch.remote ? (
            <span className={uiClasses.treeItemMeta}>{ctx.upstream}</span>
          ) : null}
        </div>
      )}
      {form ? (
        // Keys typed in the form stay out of the menu's typeahead.
        // biome-ignore lint/a11y/noStaticElementInteractions: only keeps typing inside the form
        <div className="shui-wt-menu-create" onKeyDown={(event) => event.key !== 'Escape' && event.stopPropagation()}>
          {form === 'rename' ? (
            <BranchNameForm
              initial={name}
              label={`New name for ${name}`}
              placeholder="New name (Enter to rename)"
              busy={busy}
              onSubmit={(to) => ops.renameBranch(name, to, () => onBack())}
              onCancel={() => setForm(null)}
            />
          ) : form === 'new-branch' ? (
            <BranchNameForm
              label={`New branch from ${name}`}
              placeholder={`Branch (Enter to create from ${name} and check it out)`}
              busy={busy}
              onSubmit={(branchName) => ops.newBranchHere(branchName, name, () => onBack())}
              onCancel={() => setForm(null)}
            />
          ) : (
            <BranchNameForm
              label={`New worktree from ${name}`}
              placeholder={`Branch (Enter to open a worktree for it from ${name})`}
              busy={busy}
              onSubmit={(branchName) => ops.create(branchName, () => onClose(), name)}
              onCancel={() => setForm(null)}
            />
          )}
        </div>
      ) : null}
      <List className={`${uiClasses.tree} shui-wt-menu-list`} aria-label={`Actions for ${name}`} {...actionsHover()}>
        {actions.map((action, index) => (
          <Fragment key={action.id}>
            {index > 0 && actions[index - 1].group !== action.group ? <DropdownMenuSeparator /> : null}
            <div
              className={`${uiClasses.treeItem} shui-wt-row shui-wt-action`}
              data-action={action.id}
              data-tone={action.danger ? 'alert' : undefined}
              data-open={(action.id === 'tracked' && tracked !== null) || undefined}
            >
              <button
                type="button"
                data-list-item=""
                className="shui-wt-row-main"
                aria-disabled={busy || action.disabled !== undefined || undefined}
                title={action.disabled}
                aria-haspopup={action.id === 'tracked' ? 'menu' : undefined}
                aria-expanded={action.id === 'tracked' ? tracked !== null : undefined}
                onClick={(event) => {
                  if (busy || action.disabled !== undefined) return
                  const row = event.currentTarget.parentElement
                  // From the keyboard (Enter or Space), its first action takes the focus.
                  if (action.id === 'tracked' && flyout && row !== null) openTracked(row, event.detail === 0)
                  else act(action.id)
                }}
                onKeyDown={(event) => {
                  if (event.key !== 'ArrowRight' || action.id !== 'tracked') return
                  event.preventDefault()
                  const row = event.currentTarget.parentElement
                  if (flyout && row !== null) openTracked(row, true)
                  else act(action.id)
                }}
              >
                <span className={uiClasses.treeItemLabel}>{action.label}</span>
              </button>
              {action.id === 'tracked' ? (
                <span className={uiClasses.treeItemTrailing}>
                  <ChevronRight aria-hidden className="shui-wt-row-check" />
                </span>
              ) : null}
            </div>
          </Fragment>
        ))}
      </List>
      {tracked !== null && ctx.upstream !== null ? (
        <Flyout at={tracked.at} focus={tracked.focus} onPointerEnter={hover.cancel}>
          <BranchPage
            branch={{ name: ctx.upstream, remote: true }}
            ctx={{ ...ctx, upstream: null, checkedOutIn: null, worktree: null }}
            worktree={null}
            ops={ops}
            flyout
            onSettle={setTrackedSettled}
            busy={busy}
            onBack={() => closeTracked(true)}
            onTracked={onTracked}
            onClose={onClose}
            onShowInGit={onShowInGit}
          />
        </Flyout>
      ) : null}
    </div>
  )
}
