import { useMemo, useState } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { isAutoAcceptable } from '@/lib/backend/auto-accept-policy'
import type { FunctionEntry } from '@/lib/functions'
import { cn } from '@/lib/utils'

interface FunctionAllowlistTreeProps {
  functions: FunctionEntry[]
  /** Current allowlist (function ids). */
  allowlist: ReadonlySet<string>
  onAdd: (functionId: string) => void
  onRemove: (functionId: string) => void
  /** Empty-state hint when the catalog has no entries (still loading, etc). */
  emptyHint?: string
}

interface TreeNode {
  segment: string
  path: string
  entry?: FunctionEntry
  children: TreeNode[]
}

function buildTree(entries: ReadonlyArray<FunctionEntry>): TreeNode[] {
  const root: TreeNode = { segment: '', path: '', children: [] }
  const sorted = [...entries].sort((a, b) => a.id.localeCompare(b.id))
  for (const entry of sorted) {
    const segments = entry.id.split('::').filter(Boolean)
    if (segments.length === 0) continue
    let cursor = root
    let path = ''
    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i]
      path = path ? `${path}::${seg}` : seg
      let child = cursor.children.find((c) => c.segment === seg)
      if (!child) {
        child = { segment: seg, path, children: [] }
        cursor.children.push(child)
      }
      if (i === segments.length - 1) child.entry = entry
      cursor = child
    }
  }
  return root.children
}

interface NodeCounts {
  total: number
  allowed: number
  destructive: number
  leafIds: string[]
}

function collectCounts(
  node: TreeNode,
  allowlist: ReadonlySet<string>,
): NodeCounts {
  let total = 0
  let allowed = 0
  let destructive = 0
  const leafIds: string[] = []
  if (node.entry) {
    total += 1
    leafIds.push(node.entry.id)
    if (allowlist.has(node.entry.id)) allowed += 1
    if (!isAutoAcceptable(node.entry.id)) destructive += 1
  }
  for (const child of node.children) {
    const c = collectCounts(child, allowlist)
    total += c.total
    allowed += c.allowed
    destructive += c.destructive
    leafIds.push(...c.leafIds)
  }
  return { total, allowed, destructive, leafIds }
}

/**
 * Recursive function-id tree grouped by `::`. Branch checkboxes are
 * tri-state and toggle their subtree as a unit; destructive leaves and
 * destructive bulk-adds gate on a confirmation dialog. Pure UI — the
 * parent owns the allowlist state and add/remove callbacks.
 */
export function FunctionAllowlistTree({
  functions,
  allowlist,
  onAdd,
  onRemove,
  emptyHint = 'no functions in the catalog yet.',
}: FunctionAllowlistTreeProps) {
  const tree = useMemo(() => buildTree(functions), [functions])
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  /**
   * FIFO of destructive function ids waiting on user confirmation. The
   * modal renders queue[0]; confirm dequeues + adds, cancel clears the
   * whole queue (so a single "cancel" stops the chain rather than the
   * user having to dismiss N modals).
   */
  const [destructiveQueue, setDestructiveQueue] = useState<string[]>([])
  const pendingDestructive = destructiveQueue[0] ?? null
  const remainingDestructive = Math.max(destructiveQueue.length - 1, 0)

  function toggleBranch(path: string) {
    setExpanded((prev) => ({ ...prev, [path]: !prev[path] }))
  }

  function setSubtreeAllowed(node: TreeNode, value: boolean) {
    const { leafIds } = collectCounts(node, allowlist)
    if (value) {
      const nonDestructive = leafIds.filter((id) => isAutoAcceptable(id))
      const destructive = leafIds.filter(
        (id) => !isAutoAcceptable(id) && !allowlist.has(id),
      )
      for (const id of nonDestructive) {
        if (!allowlist.has(id)) onAdd(id)
      }
      if (destructive.length > 0) {
        setDestructiveQueue((prev) => [...prev, ...destructive])
      }
    } else {
      for (const id of leafIds) {
        if (allowlist.has(id)) onRemove(id)
      }
    }
  }

  function toggleLeaf(id: string) {
    if (allowlist.has(id)) {
      onRemove(id)
      return
    }
    if (!isAutoAcceptable(id)) {
      setDestructiveQueue((prev) => [...prev, id])
      return
    }
    onAdd(id)
  }

  if (tree.length === 0) {
    return (
      <p className="font-mono text-[12px] text-ink-faint p-4 text-center">
        {emptyHint}
      </p>
    )
  }

  return (
    <>
      <ul className="font-mono text-[12.5px]">
        {tree.map((node) => (
          <TreeBranch
            key={node.path}
            node={node}
            depth={0}
            allowlist={allowlist}
            expanded={expanded}
            onToggleBranch={toggleBranch}
            onToggleLeaf={toggleLeaf}
            onToggleSubtree={setSubtreeAllowed}
          />
        ))}
      </ul>
      <DestructiveConfirm
        functionId={pendingDestructive}
        remaining={remainingDestructive}
        onCancel={() => setDestructiveQueue([])}
        onConfirm={(id) => {
          onAdd(id)
          setDestructiveQueue((prev) => prev.slice(1))
        }}
      />
    </>
  )
}

interface TreeBranchProps {
  node: TreeNode
  depth: number
  allowlist: ReadonlySet<string>
  expanded: Record<string, boolean>
  onToggleBranch: (path: string) => void
  onToggleLeaf: (id: string) => void
  onToggleSubtree: (node: TreeNode, value: boolean) => void
}

function TreeBranch({
  node,
  depth,
  allowlist,
  expanded,
  onToggleBranch,
  onToggleLeaf,
  onToggleSubtree,
}: TreeBranchProps) {
  const counts = collectCounts(node, allowlist)
  const hasChildren = node.children.length > 0
  const isOpen = expanded[node.path] !== false && hasChildren
  const branchState: 'unchecked' | 'mixed' | 'checked' =
    counts.allowed === 0
      ? 'unchecked'
      : counts.allowed === counts.total
        ? 'checked'
        : 'mixed'

  const indent = { paddingLeft: `${depth * 14 + 4}px` }

  return (
    <li className="select-none">
      <div
        className="flex items-center gap-2 py-[2px] hover:bg-paper-2"
        style={indent}
      >
        {hasChildren ? (
          <button
            type="button"
            onClick={() => onToggleBranch(node.path)}
            aria-label={
              isOpen ? `collapse ${node.segment}` : `expand ${node.segment}`
            }
            className={cn(
              'text-ink-ghost shrink-0 w-3 transition-transform',
              isOpen && 'rotate-90',
            )}
          >
            ▸
          </button>
        ) : (
          <span className="w-3 shrink-0" aria-hidden />
        )}

        {hasChildren ? (
          <Tristate
            state={branchState}
            onClick={() => onToggleSubtree(node, branchState !== 'checked')}
            aria-label={`toggle all in ${node.path}`}
          />
        ) : node.entry ? (
          <Tristate
            state={allowlist.has(node.entry.id) ? 'checked' : 'unchecked'}
            onClick={() => onToggleLeaf(node.entry!.id)}
            aria-label={`toggle ${node.entry.id}`}
          />
        ) : null}

        <span className="flex-1 min-w-0 truncate text-ink">
          {node.segment}
          {node.entry && !isAutoAcceptable(node.entry.id) ? (
            <span className="ml-2 text-alert text-[11px]">
              · potentially destructive
            </span>
          ) : null}
        </span>

        {hasChildren ? (
          <span className="font-mono text-[11px] text-ink-faint tabular-nums shrink-0">
            {counts.allowed}/{counts.total}
          </span>
        ) : null}
      </div>

      {hasChildren && isOpen ? (
        <ul>
          {node.children.map((child) => (
            <TreeBranch
              key={child.path}
              node={child}
              depth={depth + 1}
              allowlist={allowlist}
              expanded={expanded}
              onToggleBranch={onToggleBranch}
              onToggleLeaf={onToggleLeaf}
              onToggleSubtree={onToggleSubtree}
            />
          ))}
        </ul>
      ) : null}
    </li>
  )
}

interface TristateProps {
  state: 'checked' | 'unchecked' | 'mixed'
  onClick: () => void
  'aria-label': string
}

function Tristate({ state, onClick, 'aria-label': label }: TristateProps) {
  const ariaChecked =
    state === 'checked' ? true : state === 'mixed' ? 'mixed' : false
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={ariaChecked}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      className={cn(
        'shrink-0 w-3.5 h-3.5 border flex items-center justify-center',
        'border-rule hover:border-ink transition-colors',
        state === 'checked' && 'bg-ink border-ink text-bg',
        state === 'mixed' && 'bg-ink-ghost border-ink-ghost text-bg',
      )}
    >
      <span aria-hidden className="text-[10px] leading-none">
        {state === 'checked' ? '✓' : state === 'mixed' ? '–' : ''}
      </span>
    </button>
  )
}

interface DestructiveConfirmProps {
  functionId: string | null
  /** Count of additional destructive ids still in the queue behind this one. */
  remaining: number
  onCancel: () => void
  onConfirm: (id: string) => void
}

function DestructiveConfirm({
  functionId,
  remaining,
  onCancel,
  onConfirm,
}: DestructiveConfirmProps) {
  const open = functionId !== null
  const total = remaining + (functionId ? 1 : 0)
  return (
    <Dialog open={open} onOpenChange={(v) => !v && onCancel()}>
      <DialogContent className="max-w-md">
        <DialogTitle className="text-[14px]">
          allowlist a potentially destructive function?
          {total > 1 ? (
            <span className="ml-2 text-ink-faint text-[12px] tabular-nums">
              {total - remaining}/{total}
            </span>
          ) : null}
        </DialogTitle>
        <DialogDescription className="mt-3 text-ink">
          <code className="px-1 bg-paper-2 text-accent">{functionId}</code>{' '}
          matches the potentially-destructive verb policy (write / delete / exec
          / send / credential / …). adding it means the agent will run it
          without asking while auto mode is on.
        </DialogDescription>
        {remaining > 0 ? (
          <DialogDescription className="mt-2 text-ink-faint">
            {remaining} more potentially destructive function
            {remaining === 1 ? '' : 's'} queued from your select-all. cancel to
            stop the chain.
          </DialogDescription>
        ) : null}
        <div className="mt-6 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="font-mono text-[12px] px-3 py-1 border border-rule text-ink-faint hover:text-ink hover:border-ink transition-colors"
          >
            cancel
          </button>
          <button
            type="button"
            onClick={() => functionId && onConfirm(functionId)}
            className="font-mono text-[12px] px-3 py-1 border border-alert bg-alert text-bg hover:bg-bg hover:text-alert transition-colors"
          >
            allow {functionId}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
