import { useMemo, useState } from 'react'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
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
  // Capture into a const so the narrowing survives into the onClick closure
  // (a property access like node.entry would not narrow inside the callback).
  const entry = node.entry
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
        ) : entry ? (
          <Tristate
            state={allowlist.has(entry.id) ? 'checked' : 'unchecked'}
            onClick={() => onToggleLeaf(entry.id)}
            aria-label={`toggle ${entry.id}`}
          />
        ) : null}

        <span className="flex-1 min-w-0 truncate text-ink">
          {node.segment}
          {entry && !isAutoAcceptable(entry.id) ? (
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
    // biome-ignore lint/a11y/useSemanticElements: native <input type="checkbox"> can't render the tri-state "mixed" glyph with custom styling; button + role="checkbox" + aria-checked="mixed" is the WAI-ARIA tri-state pattern
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
        'shrink-0 size-4 rounded-sm flex items-center justify-center transition-colors',
        state === 'unchecked' && 'bg-surface hover:bg-surface-hover',
        state === 'checked' && 'bg-ink text-bg',
        state === 'mixed' && 'bg-ink-ghost text-bg',
      )}
    >
      <span aria-hidden className="text-[11px] leading-none">
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
    <ConfirmDialog
      open={open}
      // The queue decides what opens next: closing only ever goes through
      // Cancel (stop the chain) or the confirm (on to the next one).
      onOpenChange={() => {}}
      title={
        total > 1
          ? `Allowlist a destructive function? (${total - remaining} of ${total})`
          : 'Allowlist a destructive function?'
      }
      description={`${functionId ?? ''} matches the destructive-action policy (write, delete, exec, send, credentials), so in auto mode the agent runs it without asking.${
        remaining > 0
          ? ` ${remaining} more from your selection ${remaining === 1 ? 'follows' : 'follow'}; Cancel stops there.`
          : ''
      }`}
      confirmLabel="Allowlist"
      tone="danger"
      onCancel={onCancel}
      onConfirm={() => {
        if (functionId) onConfirm(functionId)
      }}
    />
  )
}
