/* The Log's branches tree, laid out like the left pane of WebStorm's Log.
   Under HEAD come Local, Remote and Tags, with branches grouped in folders
   by their prefix (feat/, fix/). A branch checked out in a worktree has
   the worktree glyph, and each branch's glyph takes the same colour its
   lane has in the graph. Counts are against the upstream: ↙ is what
   arrives on a pull, ↗ what a push sends.

   It is a tree in the ARIA sense. The arrows walk the visible rows, → opens
   a group or steps into it, ← closes it or climbs to its parent, and
   typing jumps to a matching name. The search field above narrows the tree
   to matching refs, with their groups open. A click on a group's caret
   opens or closes it. */

import { SearchField, uiClasses } from '@iii-dev/console-ui'
import {
  ArrowDownLeft,
  ArrowUpRight,
  Check,
  ChevronRight,
  Crosshair,
  Folder,
  FolderGit2,
  GitBranch,
  Tag,
} from 'lucide-react'
import { type CSSProperties, useId, useMemo, useState } from 'react'
import { glyphOf } from './CommitGraph'
import type { ContextMenuAnchor } from './ContextMenu'
import type { RefTreeNode } from './git-log-window'
import { speedMarks, useRowNav } from './use-row-nav'

interface Row {
  node: RefTreeNode
  depth: number
  parent: string | null
  /** A group's children are showing. */
  open: boolean
  /** Place among its siblings, for aria-setsize/posinset. */
  size: number
  pos: number
}

const isGroup = (node: RefTreeNode): node is Extract<RefTreeNode, { children: RefTreeNode[] }> =>
  node.kind === 'section' || node.kind === 'folder'

/** The rows showing: every node whose groups are open. With a query, the
    refs matching it, inside all their groups, opened. */
function visibleRows(nodes: readonly RefTreeNode[], expanded: ReadonlySet<string>, query: string): Row[] {
  const needle = query.trim().toLowerCase()
  const matches = (node: RefTreeNode): boolean =>
    isGroup(node)
      ? node.children.some(matches)
      : node.kind === 'ref'
        ? node.ref.name.toLowerCase().includes(needle)
        : node.label.toLowerCase().includes(needle)
  const rows: Row[] = []
  const walk = (list: readonly RefTreeNode[], depth: number, parent: string | null) => {
    const shown = needle === '' ? list : list.filter(matches)
    shown.forEach((node, index) => {
      const open = isGroup(node) && (needle !== '' || expanded.has(node.id))
      rows.push({ node, depth, parent, open, size: shown.length, pos: index + 1 })
      if (open && isGroup(node)) walk(node.children, depth + 1, node.id)
    })
  }
  walk(nodes, 0, null)
  return rows
}

export function GitBranchTree({
  nodes,
  defaultBranch,
  remotes,
  expanded,
  onExpanded,
  selected,
  onSelect,
  onAct,
  onDelete,
  onMenu,
  onTap,
  onDrill,
  onRename,
}: {
  nodes: readonly RefTreeNode[]
  defaultBranch: string | null
  remotes: ReadonlySet<string>
  expanded: ReadonlySet<string>
  onExpanded(id: string, open: boolean): void
  selected: string | null
  onSelect(id: string): void
  /** Enter or a double click on a ref: open it. */
  onAct(node: RefTreeNode): void
  onDelete(node: RefTreeNode): void
  onMenu(node: RefTreeNode, anchor: ContextMenuAnchor): void
  /** A click on a row, after it is selected (a narrow pane drills in). */
  onTap?(node: RefTreeNode, open: boolean): void
  /** → on a branch (a narrow pane drills into its commits). */
  onDrill?(node: RefTreeNode): void
  /** F2 on a branch. */
  onRename?(node: RefTreeNode): void
}) {
  const [query, setQuery] = useState('')
  const rows = useMemo(() => visibleRows(nodes, expanded, query), [nodes, expanded, query])
  const domId = useId()
  const nav = useRowNav<Row>({
    items: rows,
    idOf: (row) => row.node.id,
    labelOf: (row) => row.node.label,
    domId,
    selected,
    onSelect: (id) => {
      if (id !== null) onSelect(id)
    },
    onAct: (row) => {
      if (isGroup(row.node)) onExpanded(row.node.id, !row.open)
      else onAct(row.node)
    },
    onDelete: (row) => onDelete(row.node),
    onMenu: (row, anchor) => onMenu(row.node, anchor),
    onClickRow: (row) => onTap?.(row.node, row.open),
    // A closed group or the search hides rows it keeps; one that goes away
    // is the owner's to clear.
    handOff: false,
    onKey: (event, row, index) => {
      if (event.key === 'F2' && !isGroup(row.node) && onRename) {
        event.preventDefault()
        onRename(row.node)
        return true
      }
      if (event.key === 'ArrowRight' && !isGroup(row.node) && onDrill) {
        event.preventDefault()
        onDrill(row.node)
        return true
      }
      if (event.key === 'ArrowRight' && isGroup(row.node)) {
        event.preventDefault()
        if (!row.open) onExpanded(row.node.id, true)
        else if (rows[index + 1]?.parent === row.node.id) onSelect(rows[index + 1].node.id)
        return true
      }
      if (event.key === 'ArrowLeft') {
        event.preventDefault()
        if (isGroup(row.node) && row.open) onExpanded(row.node.id, false)
        else if (row.parent !== null) onSelect(row.parent)
        return true
      }
      return false
    },
  })

  return (
    <div className="shui-git-branches" data-pane="branches">
      <SearchField
        className="shui-git-branch-search"
        value={query}
        onChange={setQuery}
        placeholder="Branch or tag"
        aria-label="Filter branches and tags"
        autoComplete="off"
        spellCheck={false}
      />
      <div
        role="tree"
        aria-label="Branches"
        className={`${uiClasses.tree} shui-git-tree`}
        data-git-focus=""
        {...nav.listProps}
      >
        {rows.map((row, index) => {
          const { node } = row
          const ref = node.kind === 'ref' ? node.ref : node.kind === 'head' ? node.ref : null
          const glyph = ref !== null && ref.kind !== 'tag' ? glyphOf(ref.name, defaultBranch, remotes) : null
          const Icon =
            node.kind === 'head'
              ? Crosshair
              : node.kind === 'folder'
                ? Folder
                : ref?.kind === 'tag'
                  ? Tag
                  : ref?.worktree
                    ? FolderGit2
                    : GitBranch
          return (
            // biome-ignore lint/a11y/useFocusableInteractive: the tree holds focus and names this row through aria-activedescendant
            <div
              key={node.id}
              role="treeitem"
              className={`${uiClasses.treeItem} shui-git-node`}
              data-kind={node.kind}
              data-remote={ref?.kind === 'remote' || undefined}
              aria-level={row.depth + 1}
              aria-setsize={row.size}
              aria-posinset={row.pos}
              aria-expanded={isGroup(node) ? row.open : undefined}
              aria-current={ref?.current ? 'true' : undefined}
              title={ref?.worktree ? `${ref.name}\nworktree ${ref.worktree}` : ref?.name}
              style={{ '--iii-ui-tree-depth': row.depth } as CSSProperties}
              {...nav.rowProps(index)}
            >
              {isGroup(node) ? (
                // biome-ignore lint/a11y/useKeyWithClickEvents lint/a11y/noStaticElementInteractions: the keys are the tree's (← →, Enter)
                <span
                  className={`${uiClasses.treeItemIcon} shui-git-caret`}
                  data-open={row.open || undefined}
                  onClick={(event) => {
                    // The caret opens or closes; the row's click selects.
                    event.stopPropagation()
                    onExpanded(node.id, !row.open)
                  }}
                >
                  <ChevronRight aria-hidden />
                </span>
              ) : null}
              {node.kind !== 'section' ? (
                <span
                  className={uiClasses.treeItemIcon}
                  data-color={glyph !== null && glyph !== 'faint' ? glyph : undefined}
                >
                  <Icon aria-hidden />
                </span>
              ) : null}
              <span className={uiClasses.treeItemLabel}>
                {speedMarks(node.label, nav.query).map((part, at) =>
                  part.hit ? (
                    <mark key={at} className="shui-git-hit">
                      {part.text}
                    </mark>
                  ) : (
                    part.text
                  ),
                )}
              </span>
              <span className={uiClasses.treeItemTrailing}>
                {ref?.behind ? (
                  <span className="shui-git-count" data-way="in" title={`${ref.behind} to pull from ${ref.upstream}`}>
                    <ArrowDownLeft aria-hidden />
                    {ref.behind}
                    <span className="shui-sr-only"> to pull</span>
                  </span>
                ) : null}
                {ref?.ahead ? (
                  <span className="shui-git-count" title={`${ref.ahead} to push to ${ref.upstream}`}>
                    <ArrowUpRight aria-hidden />
                    {ref.ahead}
                    <span className="shui-sr-only"> to push</span>
                  </span>
                ) : null}
                {ref?.gone ? (
                  <span className="shui-git-count" data-way="gone" title={`${ref.upstream} is gone`}>
                    gone
                  </span>
                ) : null}
                {ref?.current && node.kind === 'ref' ? <Check aria-hidden className="shui-git-current" /> : null}
              </span>
            </div>
          )
        })}
      </div>
      {nav.query !== '' ? <span className="shui-git-speed">{nav.query}</span> : null}
    </div>
  )
}
