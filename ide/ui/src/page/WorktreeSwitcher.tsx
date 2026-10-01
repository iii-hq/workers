/* The branch chip and its menu, beside a folder: the chat's, in the
   composer's project strip, and the IDE's, in its header. The chip names
   the branch the folder is on (● with uncommitted changes), with a worktree
   icon when the folder is a linked worktree. The menu lists the
   repository's worktrees, to switch into, merge or remove, then its
   branches without one, each opening its own (worktrunk's `wt switch`), and
   makes a worktree for a new branch from what is typed (`wt switch -c`).
   Whatever moves, the chat and the IDE beside it follow. */

import type { ComposerControlProps, Host, LiveAnnouncement } from '@iii-dev/console-ui'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  IconButton,
  List,
  LiveRegion,
  SearchField,
  uiClasses,
} from '@iii-dev/console-ui'
import {
  AlertCircle,
  Check,
  ChevronDown,
  ChevronRight,
  FolderGit2,
  GitBranch,
  GitMerge,
  Loader2,
  Plus,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import {
  type CSSProperties,
  type KeyboardEvent,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react'
import { workspaceValidate } from './coder'
import { basename } from './paths'
import { useWorktreeOps, type WorktreesPage } from './use-worktree-ops'
import {
  DeleteBranchDialog,
  type MergeDraft,
  MergeForm,
  mergeable,
  NewWorktreeForm,
  RemoveWorktreeDialog,
  WARNING,
  worktreeMarks,
} from './WorktreeForms'
import { type Head, useHead } from './worktree-head'
import {
  type Branch,
  branchOf,
  groupBranches,
  type Worktree,
  type WorktreeList,
  worktreeAt,
  worktreePathFor,
} from './worktrees'

/** The chat as the page the worktree operations move: its folder is the
    page's root, and moving the page asks the chat to take a folder the
    worker validated, as the folder picker does. */
function useChatPage(
  host: Host,
  sessionId: string,
  dir: string | null,
  streaming: boolean,
  locked: boolean,
): WorktreesPage {
  const live = useRef({ sessionId, dir, streaming, locked, mounted: false })
  live.current.sessionId = sessionId
  live.current.dir = dir
  live.current.streaming = streaming
  live.current.locked = locked
  useEffect(() => {
    live.current.mounted = true
    return () => {
      live.current.mounted = false
    }
  }, [])
  return useMemo<WorktreesPage>(() => {
    const request = (path: string): boolean =>
      live.current.mounted &&
      // The folder stays put while the composer is locked (a turn runs, the
      // harness is away), even for an operation that started before: that
      // one keeps its worktree and says so.
      !live.current.locked &&
      (host.chat?.requestWorkingDirectoryChange?.({ sessionId: live.current.sessionId, path }) ?? false)
    return {
      mounted: () => live.current.mounted,
      root: () => live.current.dir,
      chatDir: () => live.current.dir,
      sessionId: () => live.current.sessionId,
      unsaved: () => [],
      turnActive: () => live.current.streaming,
      changed: () => {},
      moveIde: async (path) => {
        const valid = await workspaceValidate(host, path).then(
          (result) => result.path,
          () => null,
        )
        const moved = valid !== null && request(valid)
        return { moved, chatDir: moved ? valid : live.current.dir }
      },
      moveChat: request,
    }
  }, [host])
}

function describe(head: Head): string {
  const parts = [head.branch ? `branch ${head.branch}` : 'detached HEAD']
  if (head.dirty) parts.push('uncommitted changes')
  parts.push(head.linked ? `worktree ${head.worktree}` : 'main worktree')
  return parts.join(' · ')
}

// A phone keyboard popping up on every tap hides the list it opened for; a
// menu opened from a keyboard still focuses the filter.
const FINE_POINTER = typeof window !== 'undefined' && window.matchMedia?.('(pointer: fine)').matches

// ponytail: the 50 most recently committed branches without a worktree; the
// filter reaches the others. Virtualize the list if scrolling them all is wanted.
const BRANCH_ROWS = 50

/** `entries` within the cap: every worktree, and branches without one up to
    BRANCH_ROWS; `hidden` counts the branches left out. */
function capped(entries: readonly Entry[]): { shown: Entry[]; hidden: number } {
  const shown: Entry[] = []
  let loose = 0
  for (const entry of entries) {
    if (entry.wt === null && ++loose > BRANCH_ROWS) continue
    shown.push(entry)
  }
  return { shown, hidden: Math.max(0, loose - BRANCH_ROWS) }
}

/** A menu row: a local branch with its worktree, if it has one, or a
    worktree no branch names (detached, bare). */
interface Entry {
  name: string
  branch: Branch | null
  wt: Worktree | null
}

/** Every branch, the default one first and then the most recently committed,
    each with its worktree; then the worktrees no listed branch names. */
function entriesOf(list: WorktreeList): Entry[] {
  // Git can check one branch out twice (`worktree add --force`): the first
  // worktree, the main one if it is among them, gets the branch's row.
  const byBranch = new Map<string | null, Worktree>()
  for (const wt of list.worktrees) if (!byBranch.has(branchOf(wt))) byBranch.set(branchOf(wt), wt)
  const entries: Entry[] = list.branches.map((branch) => ({
    name: branch.name,
    branch,
    wt: byBranch.get(branch.name) ?? null,
  }))
  const listed = new Set(list.branches.map((branch) => branch.name))
  for (const wt of list.worktrees) {
    const own = branchOf(wt)
    if (own === null || !listed.has(own) || byBranch.get(own) !== wt)
      entries.push({ name: own ?? '', branch: null, wt })
  }
  const first = entries.findIndex((entry) => entry.name === list.defaultBranch)
  if (first > 0) entries.unshift(...entries.splice(first, 1))
  return entries
}

/** The composer control: the menu for the chat's own folder. */
export function createWorktreeSwitcher(host: Host) {
  return function WorktreeSwitcher({ sessionId, isStreaming, workingDir, locked }: ComposerControlProps) {
    const dir = workingDir ?? null
    // Like the folder beside it: no moving the chat while the composer is locked.
    const frozen = locked ?? isStreaming
    const page = useChatPage(host, sessionId, dir, isStreaming, frozen)
    return <WorktreeMenu host={host} dir={dir} page={page} frozen={frozen} rereadKey={String(isStreaming)} side="top" />
  }
}

export interface WorktreeMenuProps {
  host: Host
  /** The folder the chip describes: the chat's, or the IDE's. */
  dir: string | null
  /** What the menu's operations move. */
  page: WorktreesPage
  /** Nothing moves while it is set. */
  frozen?: boolean
  /** Changes when the folder's branch may have changed under it, as when a turn ends. */
  rereadKey?: string
  /** Where the menu opens: above the composer, below the IDE's header. */
  side: 'top' | 'bottom'
}

// Memoized: the IDE's header and the chat's composer render it on every
// render of theirs, with props that seldom change.
export const WorktreeMenu = memo(function WorktreeMenu({
  host,
  dir,
  page,
  frozen = false,
  rereadKey = '',
  side,
}: WorktreeMenuProps) {
  const [open, setOpen] = useState(false)
  const ops = useWorktreeOps(host, dir, page, open, 'menu')
  // Refresh re-reads the chip too, even when it brings no focus change.
  const [refreshes, setRefreshes] = useState(0)
  const head = useHead(host, dir, `${rereadKey}:${refreshes}`)
  const [query, setQuery] = useState('')
  const [merging, setMerging] = useState<MergeDraft | null>(null)
  const [creating, setCreating] = useState(false)
  // The `prefix/` groups the user opened or closed, per repository; they stay
  // that way across openings of the menu.
  const [toggledGroups, setToggledGroups] = useState<ReadonlyMap<string, boolean>>(() => new Map())
  // An outcome that lands while the menu is closed marks the chip until
  // the menu shows it; every outcome is announced.
  const [unseen, setUnseen] = useState(false)
  const [announcement, setAnnouncement] = useState<LiveAnnouncement | null>(null)
  const wrapRef = useRef<HTMLSpanElement>(null)
  const openRef = useRef(open)
  openRef.current = open
  const noteSeqRef = useRef(ops.noteSeq)
  const byKeyboard = useRef(false)
  const filterRef = useRef<HTMLInputElement>(null)
  const cancelRemove = useRef(ops.cancelRemove)
  cancelRemove.current = () => {
    ops.cancelRemove()
    ops.cancelDeleteBranch()
  }
  const hidden = dir === null || head === null
  // Worked out again only when the list is, not on each render the chip's
  // re-reads and the operation notes bring, open or closed.
  const entries = useMemo(() => (ops.list === null ? [] : entriesOf(ops.list)), [ops.list])

  // Frozen (the composer locked), the menu closes: the chat's folder stays
  // put until it unlocks.
  useEffect(() => {
    if (frozen) setOpen(false)
  }, [frozen])
  // Hidden (the chat left the repository), the menu and a removal waiting
  // for confirmation go, rather than pop back up when the chip returns.
  useEffect(() => {
    if (!hidden) return
    setOpen(false)
    cancelRemove.current()
  }, [hidden])
  // Only the view that asked calls attention to the outcome; the others
  // show it inline.
  useEffect(() => {
    if (ops.noteSeq === noteSeqRef.current) return
    noteSeqRef.current = ops.noteSeq
    const text = ops.note
    if (text === null || !ops.noteIsMine) return
    setAnnouncement((previous) => ({
      seq: (previous?.seq ?? 0) + 1,
      text,
      urgency: WARNING.test(text) ? 'assertive' : 'polite',
    }))
    if (!openRef.current) setUnseen(true)
  }, [ops.noteSeq, ops.note, ops.noteIsMine])
  // The current worktree can sit far down a long list: bring it into view
  // each time the list shows.
  const listEl = useRef<HTMLDivElement | null>(null)
  const listRef = useCallback((element: HTMLDivElement | null) => {
    listEl.current = element
    element?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: 'nearest' })
  }, [])

  if (hidden) return null

  const { list, busy, note } = ops
  const target = list?.defaultBranch ?? null
  const here = list === null ? null : worktreeAt(list.worktrees, dir)
  const needle = query.trim()
  const lowered = needle.toLowerCase()
  // A folder named `<repo>.<branch>` only repeats the branch, and it is not
  // shown; the main one only repeats the repository's name. Neither is
  // matched, so a name that happens to be in the repository's does not
  // match every row.
  const conventional = (wt: Worktree) => {
    const own = branchOf(wt)
    return list !== null && !wt.main && own !== null && wt.path === worktreePathFor(list.worktrees[0].path, own)
  }
  const matches = entries.filter(
    (entry) =>
      lowered === '' ||
      entry.name.toLowerCase().includes(lowered) ||
      (entry.wt !== null &&
        !entry.wt.main &&
        !conventional(entry.wt) &&
        basename(entry.wt.path).toLowerCase().includes(lowered)),
  )
  const mainPath = list?.worktrees[0]?.path ?? ''
  // Typing a name no branch has offers a worktree for it, last.
  const creatable = list !== null && needle !== '' && !entries.some((entry) => entry.name === needle)
  const newFolder = list === null ? '' : basename(worktreePathFor(mainPath, needle))
  const unseenWarning = unseen && note !== null && WARNING.test(note)

  const focusChip = () =>
    requestAnimationFrame(() => wrapRef.current?.querySelector<HTMLElement>('.shui-wt-chip')?.focus())
  const onOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) return
    setQuery('')
    setMerging(null)
    setCreating(false)
    setUnseen(false)
  }
  const switchTo = (wt: Worktree) => {
    if (wt.prunable || wt.bare) return
    if (wt.path !== here?.path) ops.switchTo(wt)
    setOpen(false)
  }
  // Created means moved in: the menu closes as a switch does. A failure
  // keeps it open, with its note.
  const createFor = (branch: string) =>
    ops.create(branch, () => {
      setQuery('')
      setOpen(false)
    })
  // A worktree is switched into; a branch without one gets one.
  const usable = (entry: Entry) => entry.wt === null || (!entry.wt.prunable && !entry.wt.bare)
  const pick = (entry: Entry) => (entry.wt ? switchTo(entry.wt) : createFor(entry.name))
  // Enter takes the name typed, else the first row listed other than the
  // current one, else makes a worktree for a new branch of that name.
  const enter = () => {
    // The name typed, if it is listed: a worktree whose folder is gone stops
    // Enter there rather than let it fall through to some other branch.
    const named = matches.find((entry) => entry.name === needle)
    if (named) return usable(named) ? pick(named) : undefined
    const first = matches.find((entry) => usable(entry) && (entry.wt === null || entry.wt.path !== here?.path))
    if (first) return pick(first)
    if (creatable) createFor(needle)
  }
  const onFilterKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      listEl.current?.querySelector<HTMLElement>('[data-list-item]:not([disabled])')?.focus()
    } else if (
      event.key === 'Enter' &&
      // Safari commits a composition with an Enter that is no longer composing.
      !event.nativeEvent.isComposing &&
      event.nativeEvent.keyCode !== 229 &&
      needle !== '' &&
      !busy
    ) {
      event.preventDefault()
      enter()
    }
    // The menu's typeahead would take every letter from the field; Escape
    // still reaches the menu.
    if (event.key !== 'Escape') event.stopPropagation()
  }
  // Rows are the console's compact tree: one line each, the main button
  // (switch, create or toggle) spanning the whole row, and the actions on
  // its trailing edge beside that button, never inside it.
  const depthOf = (depth: number) => ({ '--iii-ui-tree-depth': depth }) as CSSProperties
  const marksOf = (dirty: boolean, marks: readonly string[]) =>
    dirty || marks.length > 0 ? (
      <span className="shui-wt-row-marks">
        {dirty ? <span className="shui-wt-row-dirty" /> : null}
        {marks.join(' ')}
      </span>
    ) : null
  // One row per branch. The worktree icon marks a branch with a folder of its
  // own, which a click switches into; a click on any other branch makes it
  // one. Inside its group a row shows the rest of the name, and is still
  // read out in full.
  const entryRow = (entry: Entry, depth: number) => {
    const { wt } = entry
    const shown = depth > 0 ? entry.name.slice(entry.name.indexOf('/') + 1) : entry.name
    if (wt === null) {
      const marks = [
        entry.branch?.ahead ? `↑${entry.branch.ahead}` : '',
        entry.branch?.behind ? `↓${entry.branch.behind}` : '',
      ].filter(Boolean)
      // Merged into, and deleted from, the default branch without a checkout.
      const own = target !== null && entry.name !== target
      const key = `branch:${entry.name}`
      return (
        <div key={key} className="shui-wt-row-with-form">
          <div className={`${uiClasses.treeItem} shui-wt-row`} style={depthOf(depth)}>
            <button
              type="button"
              data-list-item=""
              className="shui-wt-row-main"
              aria-disabled={busy || undefined}
              aria-label={[entry.name, ...marks].join(', ')}
              title={`${entry.name}\nNo worktree yet: opens one at ${worktreePathFor(mainPath, entry.name)}`}
              onClick={() => !busy && createFor(entry.name)}
            >
              <span className={uiClasses.treeItemIcon}>
                <GitBranch aria-hidden />
              </span>
              <span className={uiClasses.treeItemLabel}>{shown}</span>
              {marksOf(false, marks)}
            </button>
            {own ? (
              <span className={uiClasses.treeItemActions}>
                <button
                  type="button"
                  className={uiClasses.treeItemAction}
                  aria-label={`Merge ${entry.name} into ${target}`}
                  title={`Merge into ${target}`}
                  aria-disabled={busy || undefined}
                  onClick={() => !busy && openBranchMerge(entry.name)}
                >
                  <GitMerge aria-hidden />
                </button>
                <button
                  type="button"
                  className={uiClasses.treeItemAction}
                  data-tone="alert"
                  aria-label={`Delete ${entry.name}`}
                  title="Delete the branch"
                  aria-disabled={busy || undefined}
                  onClick={() => {
                    if (busy) return
                    // The confirmation is a dialog of its own: the menu gives way.
                    setOpen(false)
                    ops.askDeleteBranch(entry.name)
                  }}
                >
                  <Trash2 aria-hidden />
                </button>
              </span>
            ) : null}
          </div>
          {merging?.path === key && target ? (
            // Keys typed in the form stay out of the menu's typeahead.
            // biome-ignore lint/a11y/noStaticElementInteractions: only keeps typing inside the form
            <div
              className="shui-wt-menu-merge"
              onKeyDown={(event) => event.key !== 'Escape' && event.stopPropagation()}
            >
              <MergeForm
                draft={merging}
                target={target}
                busy={busy}
                worktree={false}
                onChange={setMerging}
                onMerge={() => ops.mergeBranch(entry.name, merging, () => setMerging(null))}
                onCancel={() => setMerging(null)}
              />
            </div>
          ) : null}
        </div>
      )
    }
    const current = wt.path === here?.path
    const { marks: written, titles } = worktreeMarks(wt, target)
    // The dirty mark is drawn as a dot, as on the chip.
    const marks = written.filter((mark) => mark !== '●')
    const folder = conventional(wt) ? null : basename(wt.path)
    const name = entry.name === '' ? (wt.bare ? 'bare' : 'detached') : entry.name
    const removable = !wt.main && !wt.bare
    return (
      <div key={wt.path} className="shui-wt-row-with-form">
        <div
          className={`${uiClasses.treeItem} shui-wt-row`}
          data-selected={current || undefined}
          style={depthOf(depth)}
        >
          <button
            type="button"
            data-list-item=""
            className="shui-wt-row-main"
            disabled={wt.prunable || wt.bare}
            aria-disabled={busy || undefined}
            aria-current={current ? 'true' : undefined}
            aria-label={[name, folder, wt.dirty ? 'uncommitted changes' : null, ...marks].filter(Boolean).join(', ')}
            title={titles.join('\n')}
            onClick={() => !busy && switchTo(wt)}
          >
            <span className={uiClasses.treeItemIcon}>
              <FolderGit2 aria-hidden />
            </span>
            <span className={uiClasses.treeItemLabel}>{entry.name === '' ? name : shown}</span>
            {marksOf(wt.dirty === true, marks)}
          </button>
          <span className={uiClasses.treeItemTrailing}>
            {folder ? <span className={uiClasses.treeItemMeta}>{folder}</span> : null}
            {current ? <Check aria-hidden className="shui-wt-row-check" /> : null}
          </span>
          {/* The actions are the row's own item, not the trailing's: that one
              gives way to a long name, and they must not be squeezed out. */}
          {mergeable(wt, target) || removable ? (
            <span className={uiClasses.treeItemActions}>
              {mergeable(wt, target) ? (
                <button
                  type="button"
                  className={uiClasses.treeItemAction}
                  aria-label={`Merge ${wt.branch} into ${target}`}
                  title={`Merge into ${target}`}
                  aria-disabled={busy || undefined}
                  onClick={() => !busy && openMerge(wt)}
                >
                  <GitMerge aria-hidden />
                </button>
              ) : null}
              {removable ? (
                <button
                  type="button"
                  className={uiClasses.treeItemAction}
                  data-tone="alert"
                  aria-label={wt.prunable ? `Prune ${basename(wt.path)}` : `Remove ${basename(wt.path)}`}
                  title={wt.prunable ? 'Prune the worktree' : 'Remove the worktree'}
                  aria-disabled={busy || undefined}
                  onClick={() => {
                    if (busy) return
                    // The confirmation is a dialog of its own: the menu gives way.
                    setOpen(false)
                    ops.askRemove(wt)
                  }}
                >
                  <Trash2 aria-hidden />
                </button>
              ) : null}
            </span>
          ) : null}
        </div>
        {merging?.path === wt.path && target ? (
          // Keys typed in the form stay out of the menu's typeahead.
          // biome-ignore lint/a11y/noStaticElementInteractions: only keeps typing inside the form
          <div className="shui-wt-menu-merge" onKeyDown={(event) => event.key !== 'Escape' && event.stopPropagation()}>
            <MergeForm
              draft={merging}
              target={target}
              busy={busy}
              onChange={setMerging}
              onMerge={() => ops.merge(wt, merging, () => setMerging(null))}
              onCancel={() => setMerging(null)}
            />
          </div>
        ) : null}
      </div>
    )
  }
  const more = (key: string, count: number, depth: number, where = '') => (
    <p key={key} className="shui-wt-menu-note shui-wt-menu-more" style={depthOf(depth)}>
      {count} more{where}; type to find them
    </p>
  )
  // Unfiltered, branches sit in their `prefix/` groups, closed until opened
  // (the current worktree's group starts open); a filter lists its matches
  // flat, by full name.
  const rows = (): ReactNode[] => {
    if (lowered !== '') {
      // The name typed goes first: past the cap it could be neither seen nor made.
      const ordered = [...matches].sort((a, b) => Number(b.name === needle) - Number(a.name === needle))
      const { shown, hidden } = capped(ordered)
      const out = shown.map((entry) => entryRow(entry, 0))
      return hidden > 0 ? [...out, more('more', hidden, 0)] : out
    }
    const out: ReactNode[] = []
    const groups = groupBranches(matches)
    const lone = capped(groups.flatMap((group) => (group.prefix === null ? group.branches : [])))
    const loneShown = new Set(lone.shown)
    for (const group of groups) {
      const { prefix } = group
      if (prefix === null) {
        if (loneShown.has(group.branches[0])) out.push(entryRow(group.branches[0], 0))
        continue
      }
      // Open as toggled here, for this repository; else open when it holds the
      // folder the menu is for.
      const key = `${mainPath}\0${prefix}`
      const opened =
        toggledGroups.get(key) ?? (here !== null && group.branches.some((entry) => entry.wt?.path === here.path))
      const toggle = () => setToggledGroups((toggled) => new Map(toggled).set(key, !opened))
      out.push(
        <div key={`group:${prefix}`} className={`${uiClasses.treeItem} shui-wt-row`} style={depthOf(0)}>
          <button
            type="button"
            data-list-item=""
            className="shui-wt-row-main"
            aria-expanded={opened}
            aria-label={`${prefix}, ${group.branches.length} branches`}
            onClick={toggle}
            // As in a tree: right opens the group, left closes it.
            onKeyDown={(event) => {
              if ((event.key === 'ArrowRight' && !opened) || (event.key === 'ArrowLeft' && opened)) {
                event.preventDefault()
                toggle()
              }
            }}
          >
            <span className={`${uiClasses.treeItemIcon} shui-wt-row-caret`} data-open={opened || undefined}>
              <ChevronRight aria-hidden />
            </span>
            <span className={uiClasses.treeItemLabel}>{prefix}</span>
          </button>
          <span className={uiClasses.treeItemTrailing}>
            <span className={uiClasses.treeItemMeta}>{group.branches.length}</span>
          </span>
        </div>,
      )
      if (!opened) continue
      const inGroup = capped(group.branches)
      out.push(...inGroup.shown.map((entry) => entryRow(entry, 1)))
      if (inGroup.hidden > 0) out.push(more(`more:${prefix}`, inGroup.hidden, 1, ` in ${prefix}/`))
    }
    if (lone.hidden > 0) out.push(more('more', lone.hidden, 0))
    return out
  }
  const openBranchMerge = (branch: string) => {
    const key = `branch:${branch}`
    setMerging({ path: key, squash: true, message: '' })
    void ops
      .branchMergeMessage(branch)
      .then((message) =>
        setMerging((draft) => (draft?.path === key && draft.message === '' ? { ...draft, message } : draft)),
      )
  }
  const openMerge = (wt: Worktree) => {
    setMerging({ path: wt.path, squash: true, message: '' })
    void ops
      .mergeMessage(wt)
      .then((message) =>
        setMerging((draft) => (draft?.path === wt.path && draft.message === '' ? { ...draft, message } : draft)),
      )
  }

  return (
    <span className="shui-wt-switcher" data-side={side} ref={wrapRef}>
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <DropdownMenuTrigger
          className="shui-wt-chip"
          disabled={frozen}
          aria-haspopup="dialog"
          onPointerDown={() => {
            byKeyboard.current = false
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ' || event.key === 'ArrowDown') byKeyboard.current = true
          }}
          aria-busy={busy || undefined}
          aria-label={`${describe(head)}${unseenWarning ? `. ${note}` : ''}. Switch, create, merge or remove worktrees`}
          title={unseenWarning ? `${describe(head)}\n${note}` : describe(head)}
        >
          {/* The folder is always a worktree's; the title says which one. */}
          <FolderGit2 aria-hidden className="shui-wt-chip-icon" />
          <span className="shui-wt-chip-branch">{head.branch ?? 'detached'}</span>
          {head.dirty ? <span className="shui-wt-chip-dirty" aria-hidden /> : null}
          {busy ? (
            <Loader2 aria-hidden className={`shui-wt-chip-icon ${uiClasses.spin}`} />
          ) : unseenWarning ? (
            <AlertCircle aria-hidden className="shui-wt-chip-icon shui-wt-chip-warn" />
          ) : (
            <ChevronDown aria-hidden className="shui-wt-chip-icon shui-wt-chip-caret" />
          )}
        </DropdownMenuTrigger>
        <DropdownMenuContent
          side={side}
          align="start"
          className="shui-wt-menu"
          aria-label="Branches"
          // It holds a search field, rows and forms rather than menu items.
          role="dialog"
          aria-orientation={undefined}
          // The filter takes focus through the trap Radix sets up, rather than
          // autoFocus before it, which Shift+Tab could slip out of.
          onOpenAutoFocus={(event) => {
            if (!(FINE_POINTER || byKeyboard.current)) return
            event.preventDefault()
            filterRef.current?.focus()
          }}
          // A control that vanishes under focus (a note replaced, a form
          // closed) leaves focus on the popup itself, where only Escape
          // works: the arrows and Tab bring it back to the field or a row.
          onKeyDown={(event) => {
            if (event.target !== event.currentTarget) return
            if (!['ArrowDown', 'ArrowUp', 'Home', 'End', 'PageUp', 'PageDown', 'Tab'].includes(event.key)) return
            event.preventDefault()
            event.currentTarget
              .querySelector<HTMLElement>('input:not([disabled]), [data-list-item]:not([disabled])')
              ?.focus()
          }}
        >
          {/* The menu keeps Tab from moving focus; let it reach the rows and their actions. */}
          {/* biome-ignore lint/a11y/noStaticElementInteractions: only lets Tab through to the controls inside */}
          <div className="shui-wt-menu-body" onKeyDown={(event) => event.key === 'Tab' && event.stopPropagation()}>
            <div className="shui-wt-menu-top">
              <SearchField
                className="shui-wt-menu-filter"
                value={query}
                onChange={setQuery}
                onKeyDown={onFilterKeyDown}
                placeholder="Switch to a branch, or name a new one…"
                aria-label="Filter branches, or name a new one"
                ref={filterRef}
                autoComplete="off"
                spellCheck={false}
              />
              <span className="shui-wt-menu-tools">
                <IconButton
                  label={`New worktree from ${target ?? 'HEAD'}`}
                  variant="ghost"
                  disabled={busy || list === null}
                  aria-expanded={creating}
                  onClick={() => setCreating((value) => !value)}
                >
                  <Plus size={16} aria-hidden />
                </IconButton>
                <IconButton
                  label="Refresh"
                  variant="ghost"
                  disabled={busy}
                  onClick={() => {
                    ops.reload()
                    setRefreshes((count) => count + 1)
                  }}
                >
                  <RefreshCw size={16} aria-hidden />
                </IconButton>
              </span>
            </div>
            {note ? (
              <p className={`shui-wt-menu-note${WARNING.test(note) ? ' warn' : ''}`} role="status">
                {note}
              </p>
            ) : null}
            {creating && list !== null ? (
              // Keys typed in the form stay out of the menu's typeahead.
              // biome-ignore lint/a11y/noStaticElementInteractions: only keeps typing inside the form
              <div
                className="shui-wt-menu-create"
                onKeyDown={(event) => event.key !== 'Escape' && event.stopPropagation()}
              >
                {/* Created means moved in: the menu closes, as with the filter. */}
                <NewWorktreeForm
                  target={target}
                  busy={busy}
                  onCreate={(branch) =>
                    ops.create(branch, () => {
                      setCreating(false)
                      setOpen(false)
                    })
                  }
                  onCancel={() => setCreating(false)}
                />
              </div>
            ) : null}
            {ops.error !== null ? (
              <p className="shui-wt-menu-note warn">{ops.error}</p>
            ) : list === null ? (
              <p className="shui-wt-menu-note">reading branches…</p>
            ) : (
              <List
                ref={listRef}
                className={`${uiClasses.tree} shui-wt-menu-list`}
                // Touch gets the tree's finger-sized rows, with actions always shown.
                data-narrow={FINE_POINTER ? undefined : ''}
                aria-label={`Branches of ${basename(mainPath)}`}
              >
                {rows()}
                {creatable ? (
                  <div className={`${uiClasses.treeItem} shui-wt-row`} style={depthOf(0)}>
                    <button
                      type="button"
                      data-list-item=""
                      className="shui-wt-row-main"
                      aria-disabled={busy || undefined}
                      title={`${newFolder}, a new branch from ${target ?? 'HEAD'}`}
                      onClick={() => !busy && createFor(needle)}
                    >
                      <span className={uiClasses.treeItemIcon}>
                        <Plus aria-hidden />
                      </span>
                      <span className={uiClasses.treeItemLabel}>New worktree: {needle}</span>
                    </button>
                    <span className={uiClasses.treeItemTrailing}>
                      <span className={uiClasses.treeItemMeta}>from {target ?? 'HEAD'}</span>
                    </span>
                  </div>
                ) : matches.length === 0 ? (
                  <p className="shui-wt-menu-note">Nothing matches.</p>
                ) : null}
              </List>
            )}
          </div>
        </DropdownMenuContent>
      </DropdownMenu>
      <DeleteBranchDialog
        deleting={ops.deletingBranch}
        target={target}
        onConfirm={() => {
          ops.confirmDeleteBranch()
          focusChip()
        }}
        onCancel={() => {
          ops.cancelDeleteBranch()
          focusChip()
        }}
      />
      <RemoveWorktreeDialog
        removing={ops.removing}
        target={target}
        onConfirm={() => {
          ops.confirmRemove()
          focusChip()
        }}
        onCancel={() => {
          ops.cancelRemove()
          focusChip()
        }}
      />
      <LiveRegion announcement={announcement} />
    </span>
  )
})
