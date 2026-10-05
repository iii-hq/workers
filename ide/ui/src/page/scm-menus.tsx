/* The Source Control panel's right-click menus, WebStorm's way: a change
   row's (a file, a folder or a group, acting on every entry under it) and
   a stash's. Changelists and Local History have no counterpart here, so
   their rows are left out. */

import {
  Archive,
  ArchiveRestore,
  Ban,
  ClipboardCopy,
  Download,
  Eraser,
  FileCode,
  FileDiff,
  FileDown,
  Folder,
  GitBranch,
  GitCommitHorizontal,
  GitCompareArrows,
  History,
  PackageOpen,
  Plus,
  RefreshCw,
  Trash2,
  Undo2,
} from 'lucide-react'
import { type GitAction, menuItems } from './ActionRail'
import type { ContextMenuItem } from './ContextMenu'
import { joinPath } from './coder'
import type { ChangeRow } from './commit-tree'
import { FileTypeIcon } from './file-type-icon'
import type { GitComparisonEntry } from './git'
import type { GitStash } from './git-log'
import { basename, dirname } from './paths'

type Entries = readonly GitComparisonEntry[]

export interface ChangeMenuContext {
  root: string
  busy: boolean
  /** Tick only these and go to the commit message. */
  commit(entries: Entries): void
  rollback(entries: Entries): void
  stash(entries: Entries): void
  open(entry: GitComparisonEntry, pin: boolean): void
  /** The working copy, in an editor tab. */
  jump(path: string): void
  copy(text: string): void
  remove(entry: GitComparisonEntry): void
  add(entries: Entries): void
  /** Root-relative; a folder ends in `/`. */
  ignore(paths: string[]): void
  savePatch(entries: Entries): void
  copyPatch(entries: Entries): void
  refresh(): void
  compare(path: string): void
  history(paths: string[]): void
}

const BUSY = 'another operation is running'

const files = (count: number) => `${count} ${count === 1 ? 'file' : 'files'}`

/** The menu's first row: what it acts on, so a folder's or a group's
    "Commit files…" says which files. */
function changeHead(row: ChangeRow): ContextMenuItem {
  if (row.kind === 'file') {
    return {
      type: 'label',
      id: 'head',
      label: basename(row.entry.path),
      icon: <FileTypeIcon path={row.entry.path} />,
      detail: dirname(row.entry.path) || undefined,
    }
  }
  return {
    type: 'label',
    id: 'head',
    label: row.label,
    icon: row.kind === 'folder' ? <Folder /> : undefined,
    detail: files(row.entries.length),
  }
}

export function changeMenu(row: ChangeRow, ctx: ChangeMenuContext): ContextMenuItem[] {
  const file = row.kind === 'file' ? row.entry : null
  const folder = row.kind === 'folder' ? row.path : null
  const entries = row.kind === 'file' ? [row.entry] : row.entries
  const tracked = entries.filter((entry) => entry.status !== 'untracked')
  const unversioned = entries.filter((entry) => entry.status === 'untracked')
  const busy = ctx.busy ? BUSY : null
  const deleted = file?.status === 'deleted' ? 'the file was deleted' : null
  // An unversioned folder goes in whole; anything else file by file.
  const ignored =
    folder !== null && row.group === 'unversioned' ? [`${folder}/`] : unversioned.map((entry) => entry.path)
  const target = file?.path ?? folder
  const copyItems: GitAction[] =
    target === null
      ? []
      : [
          { id: 'abs', label: 'Absolute path', icon: null, run: () => ctx.copy(joinPath(ctx.root, target)) },
          { id: 'rel', label: 'Path from root', icon: null, run: () => ctx.copy(target) },
          { id: 'name', label: 'File name', icon: null, applies: file !== null, run: () => ctx.copy(basename(target)) },
        ]
  const actions: GitAction[] = [
    {
      id: 'commit',
      label: entries.length === 1 ? 'Commit file…' : 'Commit files…',
      icon: <GitCommitHorizontal aria-hidden />,
      group: 'commit',
      run: () => ctx.commit(entries),
    },
    {
      id: 'rollback',
      label: 'Rollback…',
      icon: <Undo2 aria-hidden />,
      group: 'commit',
      blocked: tracked.length === 0 ? 'unversioned files have nothing to roll back' : busy,
      run: () => ctx.rollback(tracked),
    },
    {
      id: 'diff',
      label: 'Show diff',
      icon: <FileDiff aria-hidden />,
      group: 'open',
      applies: file !== null,
      run: () => file && ctx.open(file, false),
    },
    {
      id: 'diff-tab',
      label: 'Show diff in a new tab',
      icon: <FileDiff aria-hidden />,
      group: 'open',
      applies: file !== null,
      run: () => file && ctx.open(file, true),
    },
    {
      id: 'source',
      label: 'Jump to source',
      icon: <FileCode aria-hidden />,
      group: 'open',
      applies: file !== null,
      blocked: deleted,
      run: () => file && ctx.jump(file.path),
    },
    {
      id: 'copy',
      label: 'Copy path/reference',
      icon: <ClipboardCopy aria-hidden />,
      group: 'open',
      applies: target !== null,
      items: copyItems,
      run: () => undefined,
    },
    {
      id: 'delete',
      label: 'Delete…',
      icon: <Trash2 aria-hidden />,
      group: 'files',
      danger: true,
      applies: file !== null,
      blocked: deleted ?? busy,
      run: () => file && ctx.remove(file),
    },
    {
      id: 'add',
      label: 'Add to VCS',
      icon: <Plus aria-hidden />,
      group: 'files',
      blocked: unversioned.length === 0 ? 'already under version control' : busy,
      run: () => ctx.add(unversioned),
    },
    {
      id: 'ignore',
      label: 'Add to .gitignore',
      icon: <Ban aria-hidden />,
      group: 'files',
      blocked: unversioned.length === 0 ? 'only unversioned files can be ignored' : busy,
      run: () => ctx.ignore(ignored),
    },
    {
      id: 'patch',
      label: 'Create patch from local changes',
      icon: <FileDown aria-hidden />,
      group: 'patch',
      run: () => ctx.savePatch(entries),
    },
    {
      id: 'copy-patch',
      label: 'Copy as patch to clipboard',
      icon: <ClipboardCopy aria-hidden />,
      group: 'patch',
      run: () => ctx.copyPatch(entries),
    },
    {
      id: 'stash',
      label: 'Stash changes…',
      icon: <Archive aria-hidden />,
      group: 'patch',
      blocked: busy,
      run: () => ctx.stash(entries),
    },
    { id: 'refresh', label: 'Refresh', icon: <RefreshCw aria-hidden />, group: 'refresh', run: ctx.refresh },
    {
      id: 'git',
      label: 'Git',
      icon: <GitBranch aria-hidden />,
      group: 'git',
      applies: target !== null,
      run: () => undefined,
      items: [
        {
          id: 'compare',
          label: 'Compare with…',
          icon: <GitCompareArrows aria-hidden />,
          applies: file !== null,
          blocked: file?.status === 'untracked' || file?.status === 'added' ? 'it has no committed version' : null,
          run: () => file && ctx.compare(file.path),
        },
        {
          id: 'history',
          label: 'Show history',
          icon: <History aria-hidden />,
          blocked: tracked.length === 0 ? 'unversioned files have no history' : null,
          run: () => target !== null && ctx.history([target]),
        },
      ],
    },
  ]
  return [changeHead(row), { type: 'separator', id: 'sep:head' }, ...menuItems(actions)]
}

export interface StashMenuContext {
  busy: boolean
  apply(stash: GitStash, pop: boolean): void
  unstash(stash: GitStash): void
  drop(stash: GitStash): void
  clear(): void
  showDiff(stash: GitStash, pin: boolean): void
}

export function stashMenu(stash: GitStash, ctx: StashMenuContext): ContextMenuItem[] {
  const busy = ctx.busy ? BUSY : null
  const head: ContextMenuItem = {
    type: 'label',
    id: 'head',
    label: stash.message,
    icon: <Archive />,
    detail: stash.ref,
  }
  return [
    head,
    { type: 'separator', id: 'sep:head' },
    ...menuItems([
      {
        id: 'pop',
        label: 'Pop',
        icon: <ArchiveRestore aria-hidden />,
        group: 'apply',
        blocked: busy,
        run: () => ctx.apply(stash, true),
      },
      {
        id: 'apply',
        label: 'Apply',
        icon: <Download aria-hidden />,
        group: 'apply',
        blocked: busy,
        run: () => ctx.apply(stash, false),
      },
      {
        id: 'unstash',
        label: 'Unstash…',
        icon: <PackageOpen aria-hidden />,
        group: 'apply',
        blocked: busy,
        run: () => ctx.unstash(stash),
      },
      {
        id: 'drop',
        label: 'Drop…',
        icon: <Trash2 aria-hidden />,
        group: 'remove',
        danger: true,
        blocked: busy,
        run: () => ctx.drop(stash),
      },
      {
        id: 'clear',
        label: 'Clear…',
        icon: <Eraser aria-hidden />,
        group: 'remove',
        danger: true,
        blocked: busy,
        run: ctx.clear,
      },
      {
        id: 'diff',
        label: 'Show diff',
        icon: <FileDiff aria-hidden />,
        group: 'diff',
        run: () => ctx.showDiff(stash, false),
      },
      {
        id: 'diff-tab',
        label: 'Show diff in a new tab',
        icon: <FileDiff aria-hidden />,
        group: 'diff',
        run: () => ctx.showDiff(stash, true),
      },
    ]),
  ]
}
