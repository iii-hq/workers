/* The worktree operations behind both the IDE's Worktrees view and the
   chat composer's worktree switcher: one repository's worktrees, and
   switch / create / merge / remove with the checks that keep a removal from
   taking anything the user did not confirm. `page` is where the view sits:
   an IDE page, or the chat itself. One operation runs at a time across
   every view, and each shows its outcome. The git verbs live in
   `worktrees.ts`. */

import type { Host } from '@iii-dev/console-ui'
import { errorMessage } from '@iii-dev/console-ui/format'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import {
  CheckoutBlocked,
  type CheckoutMode,
  checkoutAndRebase,
  checkoutBranch,
  checkoutRemoteAndRebase,
  checkoutRemoteBranch,
  checkoutRevision,
  deleteRemoteBranch,
  deleteTag,
  mergeIntoCurrent,
  newBranchHere,
  pullInto,
  pushTag,
  RemoteDiverged,
  rebaseCurrent,
} from './branch-actions'
import { gitApplyCommitChanges, gitRestoreFrom } from './git-actions'
import { basename } from './paths'
import {
  branchMergeMessage,
  branchOf,
  branchStanding,
  checkRemovable,
  createBranch,
  createWorktree,
  deleteBranch,
  fetchAll,
  isInside,
  isIntactWorktree,
  listWorktrees,
  mergeBranch,
  mergeMessage,
  mergeWorktree,
  type PushTarget,
  pruneWorktrees,
  pushBranch,
  pushTarget,
  removeWorktree,
  renameBranch,
  switchPath,
  updateBranch,
  type Worktree,
  type WorktreeList,
  worktreeAt,
} from './worktrees'

/** Where a move left the IDE and the chat beside it. */
export interface SwitchOutcome {
  /** The page is on the requested folder. */
  moved: boolean
  /** The chat's working directory afterwards, as far as the page knows. */
  chatDir: string | null
}

/** Where a worktree view sits, as its operations drive it. An operation
    outlives the render, and the view, that started it, so everything is
    read at call time, and a page that is gone moves nothing. */
export interface WorktreesPage {
  mounted(): boolean
  /** The folder the page shows: the IDE's root, or the chat's own folder. */
  root(): string | null
  /** The working directory of the chat beside it. */
  chatDir(): string | null
  sessionId(): string | null
  /** Unsaved editor buffers, as absolute paths. */
  unsaved(): string[]
  /** An agent turn is running in that chat. */
  turnActive(): boolean
  /** Files changed on disk: a merge fast-forwarded a checkout. */
  changed(): void
  /** Move the page; `handToChat` takes the chat along when it can. */
  moveIde(path: string, handToChat: boolean): Promise<SwitchOutcome>
  /** Point the chat alone at a folder; the page stays where it is. */
  moveChat(path: string): boolean
}

// Every IDE page on screen, so a removal sees a pane it would pull a folder
// out from under.
const pages = new Set<WorktreesPage>()
export function registerWorktreesPage(page: WorktreesPage): () => void {
  pages.add(page)
  return () => {
    pages.delete(page)
  }
}

// One worktree operation at a time, across views and remounts: every
// mounted view shows it running, shows its outcome (in views of the same
// repository), and reloads when it ends.
interface Operation {
  running: boolean
  note: string | null
  /** The main worktree of the repository the note is about. */
  repo: string | null
  /** The view whose action the note answers. */
  from: View | null
  /** Counts finished operations: a re-read token. */
  epoch: number
  /** Counts notes, so the same text twice still reads as news. */
  noteSeq: number
}
let operation: Operation = { running: false, note: null, repo: null, from: null, epoch: 0, noteSeq: 0 }
const watchers = new Set<() => void>()
function setOperation(next: Partial<Operation>) {
  operation = { ...operation, ...next }
  for (const watcher of watchers) watcher()
}
/** A view on a page: the IDE's page shows a Worktrees view and its header
    menu, and each answers only for what was asked from it. */
interface View {
  page: WorktreesPage
  kind: 'view' | 'menu'
}
// The kinds of view each page has mounted: an outcome whose view closed
// before it landed falls to the page's menu.
const mountedKinds = new Map<WorktreesPage, View['kind'][]>()
function mountKind(page: WorktreesPage, kind: View['kind']): () => void {
  mountedKinds.set(page, [...(mountedKinds.get(page) ?? []), kind])
  return () => {
    const kinds = mountedKinds.get(page) ?? []
    kinds.splice(kinds.indexOf(kind), 1)
    if (kinds.length === 0) mountedKinds.delete(page)
  }
}
function tell(note: string, repo: string | null, from: View | null, next: Partial<Operation> = {}) {
  setOperation({ ...next, note, repo, from, noteSeq: operation.noteSeq + 1 })
}
/** Counts finished worktree operations, anywhere: views that read git
    themselves re-read when it moves. */
export function useWorktreeEpoch(): number {
  return useSyncExternalStore(watchOperation, () => operation.epoch)
}
function watchOperation(watcher: () => void): () => void {
  watchers.add(watcher)
  return () => {
    watchers.delete(watcher)
  }
}

/** The session that asked for an operation, and where its chat is; `leave`
    updates `chatDir` when it moves that chat. */
interface Origin {
  sessionId: string | null
  chatDir: string | null
}

export interface MergeOptions {
  squash: boolean
  message: string
}

export interface Removal {
  wt: Worktree
  /** What forcing the removal also deletes, as the user saw it; null for a plain one. */
  reason: string | null
  /** The list the removal was checked against; it runs against that one,
      even if the view has moved on to another folder since. */
  list: WorktreeList
}

/** A branch with no worktree, about to be deleted as the user was told:
    `unmerged` counts the commits the default branch lacks (0: it has them
    all; null: there is no default branch to tell). */
export interface BranchDeletion {
  branch: string
  tip: string
  unmerged: number | null
  list: WorktreeList
}

/** Several worktrees removed at once, each with what forcing its removal
    would also delete, as the user saw it. */
export interface ManyRemoval {
  worktrees: { wt: Worktree; reason: string | null }[]
  list: WorktreeList
}

/** What a checkout in place is of. A remote branch whose local branch has
    commits of its own carries what to do with them once asked. */
export type CheckoutTarget =
  | { kind: 'branch'; name: string }
  | { kind: 'remote'; name: string; resolve?: 'drop' | 'rebase' }
  | { kind: 'revision'; name: string }

/** A checkout that stopped to ask: local changes it would overwrite (Smart
    or Force Checkout), or a local branch with commits the remote branch
    lacks (drop them, or rebase them onto it). */
export type CheckoutQuestion =
  | { kind: 'overwrite'; target: CheckoutTarget; files: string[] }
  | { kind: 'diverged'; target: CheckoutTarget; branch: string; remoteBranch: string; ahead: number }

export type CommitFilesHow = 'revert' | 'cherry-pick' | 'get'
const HOW_LABEL: Readonly<Record<CommitFilesHow, string>> = {
  revert: 'revert',
  'cherry-pick': 'cherry-pick',
  get: 'get from revision',
}

export interface WorktreeOps {
  list: WorktreeList | null
  error: string | null
  busy: boolean
  /** Counts finished operations, anywhere: a re-read token. */
  epoch: number
  /** The last operation's outcome, when it concerns this repository. */
  note: string | null
  /** Counts notes; with `noteIsMine`, when a view should call attention to one. */
  noteSeq: number
  /** The note answers something asked from this view. */
  noteIsMine: boolean
  reload(): void
  switchTo(wt: Worktree): void
  /** A worktree for `branch`, created from `from` when the branch is new. */
  create(branch: string, onCreated?: () => void, from?: string): void
  /** Drops git's entries for worktrees whose folder is gone. */
  prune(): void
  /** Fetches every remote. */
  fetch(): void
  /** A branch from `start`, checked out nowhere; `onCreated` gets its name. */
  createBranch(name: string, start: string, onCreated?: (branch: string) => void): void
  renameBranch(from: string, to: string, onRenamed?: (branch: string) => void): void
  /** Fetches the branch's upstream and fast-forwards the branch to it. */
  updateBranch(branch: string): void
  /** Commit `sha`'s change to `paths` (repository-relative) in the IDE's
      working tree, uncommitted: undone (`revert`), applied again
      (`cherry-pick`), or the files as the commit left them (`get`). */
  commitFiles(how: CommitFilesHow, sha: string, paths: readonly string[]): void
  pushBranch(branch: string, target: PushTarget): void
  merge(wt: Worktree, options: MergeOptions, onMerged?: () => void): void
  /** The subjects a squash of `wt` folds together, oldest first. */
  mergeMessage(wt: Worktree): Promise<string>
  /** Opens `removing`, saying what a forced removal would also delete. */
  askRemove(wt: Worktree): void
  removing: Removal | null
  confirmRemove(): void
  cancelRemove(): void
  /** Merges a branch no worktree has into the default branch, then deletes it. */
  mergeBranch(branch: string, options: MergeOptions, onMerged?: () => void): void
  branchMergeMessage(branch: string): Promise<string>
  /** Opens `deletingBranch`, saying what the branch's deletion would lose. */
  askDeleteBranch(branch: string): void
  deletingBranch: BranchDeletion | null
  confirmDeleteBranch(): void
  cancelDeleteBranch(): void
  /** Opens `removingMany`: several worktrees removed, one confirmation for all. */
  askRemoveMany(worktrees: readonly Worktree[]): void
  removingMany: ManyRemoval | null
  confirmRemoveMany(): void
  cancelRemoveMany(): void
  /* In place, in the folder the view is for: each refuses while that folder
     has unsaved buffers or an agent at work in it. */
  /** `git switch <branch>`. */
  checkout(branch: string): void
  /** A remote branch's local branch, tracking it (made when missing), checked out. */
  checkoutRemote(remoteBranch: string): void
  /** A tag or a revision, checked out detached. */
  checkoutRevision(revision: string, onDone?: () => void): void
  /** A checkout that stopped to ask; `answerCheckout` runs it again as answered. */
  checkoutQuestion: CheckoutQuestion | null
  answerCheckout(answer: 'smart' | 'force' | 'drop' | 'rebase'): void
  cancelCheckout(): void
  /** A new branch from `start`, checked out. */
  newBranchHere(name: string, start: string, onDone?: () => void): void
  /** Checks `branch` out (a remote one through its local branch), then rebases it onto `onto`. */
  checkoutAndRebase(branch: string, onto: string, remote?: boolean): void
  /** The folder's branch rebased onto `onto`. */
  rebaseCurrent(onto: string): void
  /** `branch` merged into the folder's branch. */
  mergeIntoCurrent(branch: string): void
  /** A remote branch pulled into the folder's branch, by rebase or merge. */
  pullInto(remoteBranch: string, rebase: boolean): void
  /** Fetches every remote, then updates the folder's branch from its upstream. */
  updateProject(): void
  /** Opens `pushing`: where `branch` would push, and how many commits. */
  askPush(branch: string): void
  pushing: { branch: string; target: PushTarget } | null
  confirmPush(): void
  cancelPush(): void
  /** Opens `deletingRemote`: a branch to delete from its remote. */
  askDeleteRemote(remoteBranch: string): void
  deletingRemote: string | null
  confirmDeleteRemote(): void
  cancelDeleteRemote(): void
  /** `tag` pushed to `remote`. */
  pushTag(tag: string, remote: string): void
  /** Opens `deletingTag`. */
  askDeleteTag(tag: string): void
  deletingTag: string | null
  confirmDeleteTag(): void
  cancelDeleteTag(): void
}

/** A worktree the page can move to while `leaving` goes away. */
function usableHome(wt: Worktree, leaving: Worktree): boolean {
  return !wt.bare && !wt.prunable && !isInside(wt.path, leaving.path)
}

function within(path: string | null, dir: string): boolean {
  return path !== null && isInside(path, dir)
}

function files(count: number): string {
  return `${count} unsaved ${count === 1 ? 'file' : 'files'}`
}

/** The repository `root` sits in, read while `active` (a closed switcher
    reads nothing). `kind` tells a page's Worktrees view from its menu;
    `dirty: false` leaves out the dirty marks (a `git status` per worktree). */
export function useWorktreeOps(
  host: Host,
  root: string | null,
  page: WorktreesPage,
  active = true,
  kind: View['kind'] = 'view',
  dirty = true,
): WorktreeOps {
  const self: View = { page, kind }
  const mine = (from: View | null) =>
    from !== null &&
    from.page === page &&
    (from.kind === kind || (kind === 'menu' && !mountedKinds.get(page)?.includes(from.kind)))
  // Only a view on screen counts: the menu speaks for a hidden Git window
  // as for a closed one.
  useEffect(() => (active ? mountKind(page, kind) : undefined), [page, kind, active])
  // Kept with the root they were read for: after the root moves (to another
  // repository, say) the old list is not this view's to act on.
  const [listed, setListed] = useState<{ root: string; list: WorktreeList } | null>(null)
  const [failed, setFailed] = useState<{ root: string; message: string } | null>(null)
  // Fresh for the root it was read for, or any root inside one of its
  // worktrees (another folder of the same repository).
  const list =
    listed !== null && root !== null && (listed.root === root || worktreeAt(listed.list.worktrees, root) !== null)
      ? listed.list
      : null
  const error = failed !== null && failed.root === root ? failed.message : null
  const [removing, setRemoving] = useState<Removal | null>(null)
  const [deletingBranch, setDeletingBranch] = useState<BranchDeletion | null>(null)
  const [removingMany, setRemovingMany] = useState<ManyRemoval | null>(null)
  const [checkoutQuestion, setCheckoutQuestion] = useState<CheckoutQuestion | null>(null)
  const [deletingTag, setDeletingTag] = useState<string | null>(null)
  const [pushing, setPushing] = useState<{ branch: string; target: PushTarget } | null>(null)
  const [deletingRemote, setDeletingRemote] = useState<string | null>(null)
  // The latest removal asked for: an older check answering late must not
  // swap the worktree the open dialog names.
  const askRef = useRef(0)
  const [reloadEpoch, setReloadEpoch] = useState(0)
  const {
    running: busy,
    note,
    repo: noteRepo,
    from: noteFrom,
    epoch,
    noteSeq,
  } = useSyncExternalStore(watchOperation, () => operation)
  const seqRef = useRef(0)
  // The repository this view last listed, so a note about another one stays
  // out of it even while its list is loading or it has none.
  const listedRef = useRef<string | null>(null)

  // A new root (a session switch lands here too) re-reads the list, so the
  // current row follows the chat's worktree; so does every operation's end.
  useEffect(() => {
    if (!active || root === null) return
    const seq = ++seqRef.current
    listWorktrees(host, root, dirty)
      .then((next) => {
        if (seqRef.current !== seq) return
        listedRef.current = next.worktrees[0]?.path ?? null
        setListed({ root, list: next })
        setFailed(null)
      })
      .catch((err: unknown) => {
        if (seqRef.current !== seq) return
        setListed(null)
        setFailed({ root, message: errorMessage(err) })
      })
  }, [host, root, active, dirty, reloadEpoch, epoch])

  const perform = (
    label: string,
    action: (current: WorktreeList, origin: Origin) => Promise<string | undefined>,
    against: WorktreeList | null = list,
  ) => {
    if (against === null) {
      tell(`${label} failed: the folder changed; try again`, null, self)
      return
    }
    if (operation.running) {
      tell('another worktree operation is running; try again when it ends', null, self)
      return
    }
    const origin: Origin = { sessionId: page.sessionId(), chatDir: page.chatDir() }
    const repo = against.worktrees[0]?.path ?? null
    setOperation({ running: true, note: null, repo, from: self })
    void action(against, origin)
      .then(
        (result) => result ?? null,
        (err: unknown) => `${label} failed: ${errorMessage(err)}`,
      )
      .then((result) => {
        if (result === null) setOperation({ running: false, note: null, repo, epoch: operation.epoch + 1 })
        else tell(result, repo, self, { running: false, epoch: operation.epoch + 1 })
        for (const other of new Set([page, ...pages])) other.changed()
      })
  }

  /** An operation on the folder the view is for, in place: refused while
      that folder's worktree has unsaved buffers or an agent at work in it,
      since it rewrites the files under both. `own` is its branch, if any. */
  const inPlace = (
    label: string,
    action: (cwd: string, current: WorktreeList, own: string | null) => Promise<string | undefined>,
  ) =>
    perform(label, async (current) => {
      const cwd = page.root()
      if (cwd === null) throw new Error('the view has no folder')
      const here = worktreeAt(current.worktrees, cwd)
      const refused = here === null ? null : mergeBlocker(here)
      if (refused !== null) throw new Error(refused)
      return action(cwd, current, here === null ? null : branchOf(here))
    })
  // A checkout that has to ask first leaves no note: its question shows.
  const runCheckout = (target: CheckoutTarget, mode: CheckoutMode, onDone?: () => void) =>
    inPlace('checkout', async (cwd, current) => {
      try {
        const note =
          target.kind === 'branch'
            ? await checkoutBranch(host, cwd, target.name, mode)
            : target.kind === 'remote'
              ? await checkoutRemoteBranch(
                  host,
                  cwd,
                  target.name,
                  current.branches.map((branch) => branch.name),
                  mode,
                  target.resolve,
                )
              : await checkoutRevision(host, cwd, target.name, mode)
        onDone?.()
        return note
      } catch (err: unknown) {
        if (err instanceof CheckoutBlocked) {
          setCheckoutQuestion({ kind: 'overwrite', target, files: err.files })
          return undefined
        }
        if (err instanceof RemoteDiverged) {
          const { branch, remoteBranch, ahead } = err
          setCheckoutQuestion({ kind: 'diverged', target, branch, remoteBranch, ahead })
          return undefined
        }
        throw err
      }
    })
  const needBranch = (own: string | null): string => {
    if (own === null) throw new Error('the folder is on a detached HEAD; check a branch out first')
    return own
  }

  /** The folder in `target` matching the page's own subfolder, if it has one. */
  const pathIn = (current: WorktreeList, target: Worktree): Promise<string> => {
    const from = page.root() ?? target.path
    return switchPath(host, { ...current, current: worktreeAt(current.worktrees, from) }, target, from)
  }

  // What a forced removal would take with it right now (uncommitted work,
  // lone commits, submodules); null when a plain one does. Refusals throw.
  const lostWith = (current: WorktreeList, wt: Worktree): Promise<string | null> =>
    checkRemovable(host, current, wt, false).then(
      () => null,
      (err: unknown) => {
        if ((err as { dirty?: boolean }).dirty === true) return errorMessage(err)
        throw err
      },
    )

  // Unsaved buffers a squash would miss (it takes what is on disk), and an
  // agent still writing there, rule out merging a worktree as well.
  const mergeBlocker = (wt: Worktree): string | null => {
    let unsaved = 0
    for (const other of new Set([page, ...pages])) {
      if (other.mounted()) unsaved += other.unsaved().filter((path) => isInside(path, wt.path)).length
    }
    if (unsaved > 0) return `save or discard the ${files(unsaved)} in it first`
    if (page.turnActive() && within(page.chatDir(), wt.path)) return 'the agent is working in it'
    return null
  }

  const removalBlocker = (wt: Worktree, origin: Origin): string | null => {
    // A page that is gone moves nothing, and what it last saw is no guide to
    // where the chat is now.
    if (!page.mounted()) return 'the view it started from closed; remove it again'
    for (const other of pages) {
      if (other === page || !other.mounted()) continue
      const otherRoot = other.root()
      // A pane on the same chat's folder follows that chat out.
      const follows = other.sessionId() === page.sessionId() && otherRoot === other.chatDir()
      if (!follows && within(otherRoot, wt.path)) return 'an IDE pane is in it'
    }
    const why = mergeBlocker(wt)
    if (why !== null) return why
    // Another session came beside the view: neither its chat nor the one
    // that asked is this view's to move anymore.
    if (page.sessionId() !== origin.sessionId && (within(origin.chatDir, wt.path) || within(page.chatDir(), wt.path))) {
      return 'a chat that is no longer beside this view is in it'
    }
    return null
  }

  // Before a worktree goes, whatever is inside it moves to the default
  // branch's worktree, like worktrunk's `cd` back to main: the page (taking
  // the chat along when it is in there too) or the chat alone. `restore`
  // undoes that when git refuses the removal afterwards.
  const leave = async (
    current: WorktreeList,
    wt: Worktree,
    origin: Origin,
  ): Promise<{ left: boolean; restore: (() => Promise<unknown>) | null }> => {
    const from = page.root()
    const chat = page.chatDir()
    const chatIn = within(chat, wt.path)
    const ours = () => page.sessionId() === origin.sessionId
    if (!within(from, wt.path)) {
      if (chat === null || !chatIn) return { left: true, restore: null }
      const home = current.worktrees.find((other) => usableHome(other, wt))
      if (!home || !page.moveChat(home.path)) return { left: false, restore: null }
      if (ours()) origin.chatDir = home.path
      return {
        left: true,
        restore: async () => {
          if (ours()) page.moveChat(chat)
        },
      }
    }
    const home =
      current.worktrees.find((other) => other.branch === current.defaultBranch && usableHome(other, wt)) ??
      current.worktrees.find((other) => usableHome(other, wt))
    if (!home || from === null) return { left: false, restore: null }
    const outcome = await page.moveIde(await pathIn(current, home), chatIn)
    if (!outcome.moved) return { left: false, restore: null }
    if (chatIn && within(outcome.chatDir, wt.path)) {
      // The chat could not follow: bring the page back and keep the worktree.
      await page.moveIde(from, false)
      return { left: false, restore: null }
    }
    if (chatIn && ours()) origin.chatDir = outcome.chatDir
    // Once another session is beside the view, neither is its to move back.
    return { left: true, restore: async () => (ours() ? page.moveIde(from, chatIn) : undefined) }
  }

  // Right before git deletes the folder, everything that could have changed
  // while the page and the chat were leaving is looked at again. Panes that
  // follow the chat out get a moment to do so.
  const finalCheck = async (
    current: WorktreeList,
    wt: Worktree,
    origin: Origin,
    accepted: string | null,
  ): Promise<string | null> => {
    // The chat, and panes that follow it, show their new folder a render or
    // two after the move: wait for everything to be out before deleting.
    const stillIn = (): string | null => {
      if (!page.mounted()) return 'the view it started from closed; remove it again'
      if (page.sessionId() === origin.sessionId && within(page.chatDir(), wt.path)) return 'the chat is still in it'
      if (within(page.root(), wt.path)) return 'the IDE is still in it'
      const pane = [...pages].some((other) => other !== page && other.mounted() && within(other.root(), wt.path))
      return pane ? 'an IDE pane is still in it' : null
    }
    // ponytail: a fixed 2 s wait; whatever is slower than that keeps the worktree.
    for (let tries = 0; stillIn() !== null; tries += 1) {
      if (tries === 10 || !page.mounted()) return stillIn()
      await new Promise((resolve) => setTimeout(resolve, 200))
    }
    const why = removalBlocker(wt, origin)
    if (why !== null) return why
    const lost = await lostWith(current, wt)
    return lost !== null && lost !== accepted ? `${lost} now; remove it again to confirm` : null
  }

  // The removal did not happen after the move: go back, but only into a
  // worktree git still has, whole.
  const restoreIfIntact = async (current: WorktreeList, restore: (() => Promise<unknown>) | null, wt: Worktree) => {
    if (restore !== null && (await isIntactWorktree(host, current, wt.path))) await restore()
  }

  // The chat remembers every folder it was pointed at; a removed worktree
  // should not linger in its project list.
  const forgetProjects = async (dir: string) => {
    try {
      const listed = await host.iii.trigger<{ projects?: { path: string }[] }>('harness::projects::list', {})
      await Promise.all(
        (listed.projects ?? [])
          .filter((project) => isInside(project.path, dir))
          .map((project) => host.iii.trigger('harness::projects::delete', { path: project.path })),
      )
    } catch {
      // The list is a convenience: a failure leaves a stale row, nothing else.
    }
  }

  /** Remove `wt` once the page and the chat are out; the note for its outcome. */
  const removeNow = async (
    current: WorktreeList,
    wt: Worktree,
    origin: Origin,
    accepted: string | null,
  ): Promise<string> => {
    const name = basename(wt.path)
    const { left, restore } = await leave(current, wt, origin)
    if (!left) return `kept ${name}: the IDE or the chat could not leave it`
    const why = await finalCheck(current, wt, origin, accepted)
    if (why !== null) {
      await restoreIfIntact(current, restore, wt)
      return `kept ${name}: ${why}`
    }
    // ponytail: a session switched in while `git worktree remove` itself runs
    // still loses its folder; ade recovers a missing folder when it opens it.
    try {
      const { branchDeleted, branchError } = await removeWorktree(host, current, wt, accepted !== null)
      await forgetProjects(wt.path)
      const target = current.defaultBranch
      const branchNote =
        wt.branch === null
          ? ''
          : branchDeleted
            ? `, deleted ${wt.branch}`
            : branchError !== undefined
              ? `, but ${wt.branch} could not be deleted: ${branchError}`
              : target === null || wt.branch === target
                ? `, kept ${wt.branch}`
                : `, kept ${wt.branch} (not in ${target} yet)`
      return `removed ${name}${branchNote}`
    } catch (err: unknown) {
      await restoreIfIntact(current, restore, wt)
      throw err
    }
  }

  const remove = ({ wt, reason, list: checked }: Removal) =>
    perform(
      'remove',
      async (current, origin) => {
        const why = removalBlocker(wt, origin)
        if (why !== null) return `kept ${basename(wt.path)}: ${why}`
        const lost = await lostWith(current, wt)
        // Something appeared since the dialog asked: ask again with all of it.
        if (lost !== null && lost !== reason) {
          setRemoving({ wt, reason: lost, list: current })
          return lost
        }
        return removeNow(current, wt, origin, lost)
      },
      checked,
    )

  // A note with no repository (a refusal before anything ran) is for the
  // view that asked alone.
  const showNote =
    note !== null && (noteRepo === null ? mine(noteFrom) : noteRepo === (list?.worktrees[0]?.path ?? listedRef.current))

  return {
    list,
    error,
    busy,
    epoch,
    note: showNote ? note : null,
    noteSeq,
    noteIsMine: mine(noteFrom),
    reload: () => setReloadEpoch((value) => value + 1),
    switchTo: (wt) => {
      if (list === null) return
      const repo = list.worktrees[0]?.path ?? null
      void pathIn(list, wt)
        .then((path) => page.moveIde(path, true))
        .then(({ moved }) => {
          if (!moved) tell(`could not switch to ${basename(wt.path)}: it stayed where it was`, repo, self)
        })
    },
    create: (branch, onCreated, start) =>
      perform('create', async (current, origin) => {
        const from = page.root()
        const wt = await createWorktree(host, current, branch, start)
        onCreated?.()
        // Only the page and the session that asked for it move in.
        if (!page.mounted() || page.root() !== from || page.sessionId() !== origin.sessionId) {
          return `created ${basename(wt.path)}`
        }
        const { moved } = await page.moveIde(await pathIn(current, wt), true)
        return moved ? undefined : `created ${basename(wt.path)}; it stayed where it was`
      }),
    prune: () =>
      perform('prune', async (current) => {
        const pruned = await pruneWorktrees(host, current)
        return pruned === 0 ? 'no worktree to prune' : `pruned ${pruned} ${pruned === 1 ? 'worktree' : 'worktrees'}`
      }),
    fetch: () =>
      perform('fetch', async (current) => {
        await fetchAll(host, current)
        return 'fetched every remote'
      }),
    createBranch: (name, start, onCreated) =>
      perform('new branch', async (current) => {
        const branch = await createBranch(host, current, name, start)
        onCreated?.(branch)
        return `created ${branch}`
      }),
    renameBranch: (from, to, onRenamed) =>
      perform('rename', async (current) => {
        const branch = await renameBranch(host, current, from, to)
        onRenamed?.(branch)
        return `renamed ${from} to ${branch}`
      }),
    updateBranch: (branch) => perform('update', (current) => updateBranch(host, current, branch)),
    commitFiles: (how, sha, paths) =>
      perform(HOW_LABEL[how], async (current) => {
        const cwd = page.root() ?? current.worktrees[0]?.path ?? ''
        if (how === 'get') await gitRestoreFrom(host, cwd, sha, paths)
        else await gitApplyCommitChanges(host, cwd, sha, paths, how === 'revert')
        const what = paths.length === 1 ? basename(paths[0]) : `${paths.length} files`
        const short = sha.slice(0, 7)
        if (how === 'get') return `${what}: as of ${short}`
        return `${how === 'revert' ? 'reverted' : 'applied'} ${short}'s changes to ${what}`
      }),
    pushBranch: (branch, target) =>
      perform('push', async (current) => {
        await pushBranch(host, current, branch, target)
        return `pushed ${branch} to ${target.name}`
      }),
    merge: (wt, options, onMerged) =>
      perform('merge', async (current, origin) => {
        const refused = mergeBlocker(wt)
        if (refused !== null) throw new Error(refused)
        const { target, sha } = await mergeWorktree(host, current, wt, options)
        onMerged?.()
        const merged = `merged ${wt.branch} into ${target} (${sha.slice(0, 9)})`
        const name = basename(wt.path)
        try {
          const why = removalBlocker(wt, origin) ?? (await lostWith(current, wt))
          if (why !== null) return `${merged}; kept ${name}: ${why}`
          return `${merged}; ${await removeNow(current, wt, origin, null)}`
        } catch (err: unknown) {
          return `${merged}; could not remove ${name}: ${errorMessage(err)}`
        }
      }),
    mergeMessage: (wt) => (list === null ? Promise.resolve('') : mergeMessage(host, list, wt).catch(() => '')),
    mergeBranch: (branch, options, onMerged) =>
      perform('merge', async (current) => {
        const { target, sha, tip } = await mergeBranch(host, current, branch, options)
        onMerged?.()
        const merged = `merged ${branch} into ${target} (${sha.slice(0, 9)})`
        try {
          await deleteBranch(host, current, branch, tip)
          return `${merged}; deleted ${branch}`
        } catch (err: unknown) {
          return `${merged}; could not delete ${branch}: ${errorMessage(err)}`
        }
      }),
    branchMergeMessage: (branch) =>
      list === null ? Promise.resolve('') : branchMergeMessage(host, list, branch).catch(() => ''),
    askDeleteBranch: (branch) => {
      if (list === null) return
      const current = list
      const ask = ++askRef.current
      branchStanding(host, current, branch).then(
        ({ tip, unmerged }) => {
          if (askRef.current === ask) setDeletingBranch({ branch, tip, unmerged, list: current })
        },
        (err: unknown) => {
          if (askRef.current === ask) {
            tell(`delete failed: ${errorMessage(err)}`, current.worktrees[0]?.path ?? null, self)
          }
        },
      )
    },
    deletingBranch,
    confirmDeleteBranch: () => {
      askRef.current += 1
      const deletion = deletingBranch
      setDeletingBranch(null)
      if (deletion === null) return
      perform(
        'delete',
        async (current) => {
          await deleteBranch(host, current, deletion.branch, deletion.tip)
          return `deleted ${deletion.branch}`
        },
        deletion.list,
      )
    },
    cancelDeleteBranch: () => {
      askRef.current += 1
      setDeletingBranch(null)
    },
    askRemoveMany: (worktrees) => {
      if (list === null) return
      const current = list
      const ask = ++askRef.current
      // One that git refuses (locked, say) stays, with why; the rest are asked about.
      void Promise.allSettled(worktrees.map(async (wt) => ({ wt, reason: await lostWith(current, wt) }))).then(
        (results) => {
          if (askRef.current !== ask) return
          const checked = results.flatMap((result) => (result.status === 'fulfilled' ? [result.value] : []))
          const refused = results.flatMap((result, index) =>
            result.status === 'rejected'
              ? [`kept ${basename(worktrees[index].path)}: ${errorMessage(result.reason)}`]
              : [],
          )
          if (refused.length > 0) tell(refused.join('; '), current.worktrees[0]?.path ?? null, self)
          if (checked.length > 0) setRemovingMany({ worktrees: checked, list: current })
        },
      )
    },
    removingMany,
    confirmRemoveMany: () => {
      askRef.current += 1
      const batch = removingMany
      setRemovingMany(null)
      if (batch === null) return
      perform(
        'remove',
        async (current, origin) => {
          // One at a time, each checked again as it comes up: one that
          // can't go stays, with why, and the rest still go.
          const done: string[] = []
          let now = current
          for (const { wt, reason } of batch.worktrees) {
            const name = basename(wt.path)
            try {
              const why = removalBlocker(wt, origin)
              const lost = why === null ? await lostWith(now, wt) : null
              if (why !== null) done.push(`kept ${name}: ${why}`)
              // Something appeared since the dialog asked: that one stays.
              else if (lost !== null && lost !== reason) done.push(`kept ${name}: ${lost}`)
              else done.push(await removeNow(now, wt, origin, lost))
            } catch (err: unknown) {
              done.push(`kept ${name}: ${errorMessage(err)}`)
            }
            try {
              now = await listWorktrees(host, now.worktrees[0].path)
            } catch (err: unknown) {
              // What went already is reported; the rest wait for another try.
              done.push(`stopped: could not read the worktrees again: ${errorMessage(err)}`)
              break
            }
          }
          // One line: the count, then only the ones that stayed, with why.
          const removed = done.filter((line) => line.startsWith('removed ')).length
          const kept = done.filter((line) => !line.startsWith('removed '))
          return [`removed ${removed} ${removed === 1 ? 'worktree' : 'worktrees'}`, ...kept].join('; ')
        },
        batch.list,
      )
    },
    cancelRemoveMany: () => {
      askRef.current += 1
      setRemovingMany(null)
    },
    checkout: (branch) => runCheckout({ kind: 'branch', name: branch }, 'plain'),
    checkoutRemote: (remoteBranch) => runCheckout({ kind: 'remote', name: remoteBranch }, 'plain'),
    checkoutRevision: (revision, onDone) => runCheckout({ kind: 'revision', name: revision }, 'plain', onDone),
    checkoutQuestion,
    answerCheckout: (answer) => {
      const question = checkoutQuestion
      setCheckoutQuestion(null)
      if (question === null) return
      const { target } = question
      if (answer === 'smart' || answer === 'force') runCheckout(target, answer)
      else if (target.kind === 'remote') runCheckout({ ...target, resolve: answer }, 'plain')
    },
    cancelCheckout: () => setCheckoutQuestion(null),
    newBranchHere: (name, start, onDone) =>
      inPlace('new branch', async (cwd) => {
        const note = await newBranchHere(host, cwd, name, start)
        onDone?.()
        return note
      }),
    checkoutAndRebase: (branch, onto, remote = false) =>
      inPlace('checkout and rebase', (cwd, current) =>
        remote
          ? checkoutRemoteAndRebase(
              host,
              cwd,
              branch,
              onto,
              current.branches.map((entry) => entry.name),
            )
          : checkoutAndRebase(host, cwd, branch, onto),
      ),
    rebaseCurrent: (onto) => inPlace('rebase', (cwd, _current, own) => rebaseCurrent(host, cwd, needBranch(own), onto)),
    mergeIntoCurrent: (branch) =>
      inPlace('merge', (cwd, _current, own) => mergeIntoCurrent(host, cwd, needBranch(own), branch)),
    pullInto: (remoteBranch, rebase) =>
      inPlace('pull', (cwd, _current, own) => pullInto(host, cwd, needBranch(own), remoteBranch, rebase)),
    updateProject: () =>
      inPlace('update', async (_cwd, current, own) => {
        await fetchAll(host, current)
        const branch = needBranch(own)
        if (!current.branches.find((entry) => entry.name === branch)?.upstream) {
          return `fetched every remote; ${branch} tracks no remote branch`
        }
        return updateBranch(host, current, branch)
      }),
    askPush: (branch) => {
      if (list === null) return
      const current = list
      const ask = ++askRef.current
      pushTarget(host, current, branch).then(
        (target) => {
          if (askRef.current === ask) setPushing({ branch, target })
        },
        (err: unknown) => {
          if (askRef.current === ask) {
            tell(`push failed: ${errorMessage(err)}`, current.worktrees[0]?.path ?? null, self)
          }
        },
      )
    },
    pushing,
    confirmPush: () => {
      askRef.current += 1
      const push = pushing
      setPushing(null)
      if (push === null) return
      perform('push', async (current) => {
        await pushBranch(host, current, push.branch, push.target)
        return `pushed ${push.branch} to ${push.target.name}`
      })
    },
    cancelPush: () => {
      askRef.current += 1
      setPushing(null)
    },
    askDeleteRemote: (remoteBranch) => setDeletingRemote(remoteBranch),
    deletingRemote,
    confirmDeleteRemote: () => {
      const remoteBranch = deletingRemote
      setDeletingRemote(null)
      if (remoteBranch === null) return
      perform('delete', async (current) => {
        const cwd = page.root() ?? current.worktrees[0]?.path ?? ''
        return deleteRemoteBranch(host, cwd, remoteBranch)
      })
    },
    cancelDeleteRemote: () => setDeletingRemote(null),
    pushTag: (tag, remote) =>
      perform('push', (current) => pushTag(host, page.root() ?? current.worktrees[0]?.path ?? '', remote, tag)),
    askDeleteTag: (tag) => setDeletingTag(tag),
    deletingTag,
    confirmDeleteTag: () => {
      const tag = deletingTag
      setDeletingTag(null)
      if (tag === null) return
      perform('delete', (current) => deleteTag(host, page.root() ?? current.worktrees[0]?.path ?? '', tag))
    },
    cancelDeleteTag: () => setDeletingTag(null),
    askRemove: (wt) => {
      if (list === null) return
      const current = list
      const ask = ++askRef.current
      lostWith(current, wt).then(
        (reason) => {
          if (askRef.current === ask) setRemoving({ wt, reason, list: current })
        },
        (err: unknown) => {
          if (askRef.current === ask)
            tell(`remove failed: ${errorMessage(err)}`, current.worktrees[0]?.path ?? null, self)
        },
      )
    },
    removing,
    confirmRemove: () => {
      askRef.current += 1
      const removal = removing
      setRemoving(null)
      if (removal) remove(removal)
    },
    cancelRemove: () => {
      askRef.current += 1
      setRemoving(null)
    },
  }
}
