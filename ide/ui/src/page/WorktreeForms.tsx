/* The pieces the IDE's Worktrees view and the composer's worktree switcher
   share: a row's status marks, the squash-merge form, and the removal
   confirmation that says what a forced removal would also delete. */

import { Button, ConfirmDialog, Dialog, DialogContent, DialogDescription, DialogTitle } from '@iii-dev/console-ui'
import { GitMerge } from 'lucide-react'
import { useState } from 'react'
import { basename } from './paths'
import type { BranchDeletion, CheckoutQuestion, ManyRemoval, MergeOptions, Removal } from './use-worktree-ops'
import type { Worktree } from './worktrees'

const MOD_KEY = typeof navigator !== 'undefined' && navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'

/** An outcome note saying something was not done, which the views flag. */
export const WARNING = /failed|could not|^kept|; kept |stayed where|try again/

/** worktrunk's `wt list` status, compact: the marks shown and a longer title. */
export function worktreeMarks(wt: Worktree, target: string | null): { marks: string[]; titles: string[] } {
  const marks: string[] = []
  // The main worktree is the first row; a `main` mark would read as the branch.
  const titles: string[] = [wt.main ? `${wt.path} (main worktree)` : wt.path]
  if (wt.dirty) {
    marks.push('●')
    titles.push('uncommitted changes')
  }
  if (wt.ahead) {
    marks.push(`↑${wt.ahead}`)
    titles.push(`${wt.ahead} ahead of ${target}`)
  }
  if (wt.behind) {
    marks.push(`↓${wt.behind}`)
    titles.push(`${wt.behind} behind ${target}`)
  }
  if (wt.held) {
    marks.push(wt.held.by === 'rebase' ? 'rebasing' : 'bisecting')
    titles.push(`a ${wt.held.by} is stopped on ${wt.held.branch}`)
  }
  if (wt.locked) marks.push('locked')
  if (wt.prunable) marks.push('gone')
  return { marks, titles }
}

/** `wt` can be merged into `target` from these views. */
export function mergeable(wt: Worktree, target: string | null): target is string {
  return target !== null && !wt.main && !wt.bare && !wt.prunable && wt.branch !== null && wt.branch !== target
}

/** A branch name for a new worktree, which starts from `target` unless that
    branch exists already. Enter creates it. */
export function NewWorktreeForm({
  target,
  busy,
  onCreate,
  onCancel,
}: {
  target: string | null
  busy: boolean
  onCreate: (branch: string) => void
  onCancel: () => void
}) {
  const [branch, setBranch] = useState('')
  return (
    <form
      className="shui-scm-commit"
      onSubmit={(event) => {
        event.preventDefault()
        if (branch.trim() !== '' && !busy) onCreate(branch)
      }}
    >
      <input
        className="shui-scm-message shui-wt-input"
        value={branch}
        onChange={(event) => setBranch(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel()
        }}
        placeholder={`Branch (Enter to create${target ? ` from ${target}` : ''})`}
        aria-label="New worktree branch"
        spellCheck={false}
        // Read-only, not disabled, while it runs: focus stays in the field.
        readOnly={busy}
        // biome-ignore lint/a11y/noAutofocus: opened by the user's own click on "New worktree"
        autoFocus
      />
    </form>
  )
}

/** One branch name, for a new branch or a rename: Enter submits, Escape
    cancels. */
export function BranchNameForm({
  initial = '',
  label,
  placeholder,
  busy,
  onSubmit,
  onCancel,
}: {
  initial?: string
  label: string
  placeholder: string
  busy: boolean
  onSubmit: (name: string) => void
  onCancel: () => void
}) {
  const [name, setName] = useState(initial)
  return (
    <form
      className="shui-scm-commit"
      onSubmit={(event) => {
        event.preventDefault()
        if (name.trim() !== '' && name.trim() !== initial && !busy) onSubmit(name)
      }}
    >
      <input
        className="shui-scm-message shui-wt-input"
        value={name}
        onChange={(event) => setName(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onCancel()
        }}
        onFocus={(event) => event.currentTarget.select()}
        placeholder={placeholder}
        aria-label={label}
        spellCheck={false}
        readOnly={busy}
        // biome-ignore lint/a11y/noAutofocus: opened by the user's own pick of New branch or Rename
        autoFocus
      />
    </form>
  )
}

export interface MergeDraft extends MergeOptions {
  path: string
}

export function MergeForm({
  draft,
  target,
  busy,
  worktree = true,
  onChange,
  onMerge,
  onCancel,
}: {
  draft: MergeDraft
  target: string
  busy: boolean
  /** A worktree's merge takes its uncommitted changes too; a bare branch has none. */
  worktree?: boolean
  onChange: (draft: MergeDraft) => void
  onMerge: () => void
  onCancel: () => void
}) {
  return (
    <form
      className="shui-scm-commit"
      onSubmit={(event) => {
        event.preventDefault()
        if (!busy) onMerge()
      }}
    >
      {/* Read-only, not disabled, while it runs: focus stays where it was. */}
      <textarea
        className="shui-scm-message"
        value={draft.message}
        onChange={(event) => onChange({ ...draft, message: event.target.value })}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            if (!busy) onMerge()
          }
        }}
        placeholder={`Squash commit message (${MOD_KEY}Enter to merge into ${target})`}
        aria-label="Squash commit message"
        rows={3}
        disabled={!draft.squash}
        readOnly={busy}
      />
      <label className="shui-wt-options">
        <input
          type="checkbox"
          checked={draft.squash}
          onChange={(event) => onChange({ ...draft, squash: event.target.checked })}
        />
        {worktree ? 'Squash into one commit, uncommitted changes included' : 'Squash into one commit'}
      </label>
      <span className="shui-wt-options">
        <Button
          type="submit"
          variant="primary"
          size="sm"
          disabled={draft.squash && draft.message.trim() === ''}
          aria-disabled={busy || undefined}
        >
          <GitMerge aria-hidden />
          Merge into {target}
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
          Cancel
        </Button>
      </span>
    </form>
  )
}

export function RemoveWorktreeDialog({
  removing,
  target,
  onConfirm,
  onCancel,
}: {
  removing: Removal | null
  target: string | null
  onConfirm: () => void
  onCancel: () => void
}) {
  const name = removing === null ? '' : basename(removing.wt.path)
  return (
    <ConfirmDialog
      open={removing !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
      title={removing?.wt.prunable ? `Prune ${name}?` : removing?.reason ? `Remove ${name} anyway?` : `Remove ${name}?`}
      description={
        removing?.wt.prunable
          ? 'Its folder is already gone, so git just forgets it.'
          : removing?.reason
            ? `${removing.reason}. Removing it deletes that too. This can't be undone.`
            : `The folder is deleted. Its branch is deleted too once ${target ?? 'the default branch'} has its commits.`
      }
      confirmLabel={removing?.wt.prunable ? 'Prune worktree' : 'Remove worktree'}
      tone={removing?.reason ? 'danger' : 'default'}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  )
}

/** How many of a checkout's blocked files the dialog lists. */
const LISTED_FILES = 40

/** A checkout that stopped to ask: Smart Checkout (the local changes
    stashed across it and brought back) or Force Checkout (dropped); or, for
    a remote branch whose local branch has commits of its own, drop them or
    rebase them onto it. */
export function CheckoutQuestionDialog({
  question,
  onAnswer,
  onCancel,
}: {
  question: CheckoutQuestion | null
  onAnswer: (answer: 'smart' | 'force' | 'drop' | 'rebase') => void
  onCancel: () => void
}) {
  const files = question?.kind === 'overwrite' ? question.files : []
  return (
    <Dialog open={question !== null} onOpenChange={(open) => (open ? undefined : onCancel())}>
      <DialogContent className="shui-rollback-dialog">
        {question?.kind === 'diverged' ? (
          <>
            <DialogTitle>Check out {question.remoteBranch}?</DialogTitle>
            <DialogDescription>
              {question.branch} has {question.ahead} {question.ahead === 1 ? "commit that isn't" : "commits that aren't"}{' '}
              on {question.remoteBranch}. Drop them to reset {question.branch} to the remote, or rebase them onto it.
              Either way, {question.branch} then tracks {question.remoteBranch}.
            </DialogDescription>
            <div className="shui-rollback-actions">
              <Button variant="secondary" size="md" onClick={onCancel}>
                Cancel
              </Button>
              <Button variant="destructive" size="md" onClick={() => onAnswer('drop')}>
                Drop local commits
              </Button>
              <Button variant="primary" size="md" onClick={() => onAnswer('rebase')}>
                Rebase onto remote
              </Button>
            </div>
          </>
        ) : (
          <>
            <DialogTitle>Check out {question?.target.name ?? ''}?</DialogTitle>
            <DialogDescription>
              It would overwrite your uncommitted changes to these files. Stash and check out keeps them and brings them
              back afterwards. Discard and check out throws them away.
            </DialogDescription>
            <ul className="shui-rollback-tree shui-checkout-files">
              {files.slice(0, LISTED_FILES).map((file) => (
                <li key={file}>{file}</li>
              ))}
              {files.length > LISTED_FILES ? <li>and {files.length - LISTED_FILES} more</li> : null}
            </ul>
            <div className="shui-rollback-actions">
              <Button variant="secondary" size="md" onClick={onCancel}>
                Cancel
              </Button>
              <Button variant="destructive" size="md" onClick={() => onAnswer('force')}>
                Discard and check out
              </Button>
              <Button variant="primary" size="md" onClick={() => onAnswer('smart')}>
                Stash and check out
              </Button>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}

export function RemoveManyDialog({
  removing,
  target,
  onConfirm,
  onCancel,
}: {
  removing: ManyRemoval | null
  target: string | null
  onConfirm: () => void
  onCancel: () => void
}) {
  const worktrees = removing?.worktrees ?? []
  const losing = worktrees.filter(({ reason }) => reason !== null).length
  return (
    <ConfirmDialog
      open={removing !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
      title={`Remove ${worktrees.length} ${worktrees.length === 1 ? 'worktree' : 'worktrees'}?`}
      description={
        losing === 0
          ? `Each folder is deleted, and its branch too once ${target ?? 'the default branch'} has its commits.`
          : `${worktrees
              .filter(({ reason }) => reason !== null)
              .map(({ wt }) => basename(wt.path))
              .join(', ')} ${losing === 1 ? 'holds' : 'hold'} work that exists nowhere else. Removing ${losing === 1 ? 'it' : 'them'} deletes that work. This can't be undone.`
      }
      confirmLabel={worktrees.length === 1 ? 'Remove worktree' : 'Remove worktrees'}
      tone={losing === 0 ? 'default' : 'danger'}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  )
}

export function DeleteBranchDialog({
  deleting,
  target,
  onConfirm,
  onCancel,
}: {
  deleting: BranchDeletion | null
  target: string | null
  onConfirm: () => void
  onCancel: () => void
}) {
  const unmerged = deleting?.unmerged ?? null
  return (
    <ConfirmDialog
      open={deleting !== null}
      onOpenChange={(open) => {
        if (!open) onCancel()
      }}
      title={`Delete ${deleting?.branch ?? ''}?`}
      description={
        unmerged === 0
          ? `${target} already has its commits.`
          : unmerged === null
            ? "There's no default branch to compare with, so commits on no other branch are lost. This can't be undone."
            : `${unmerged} ${unmerged === 1 ? "commit that isn't" : "commits that aren't"} on ${target} ${unmerged === 1 ? 'is' : 'are'} lost. This can't be undone.`
      }
      confirmLabel="Delete branch"
      tone={unmerged === 0 ? 'default' : 'danger'}
      onConfirm={onConfirm}
      onCancel={onCancel}
    />
  )
}
