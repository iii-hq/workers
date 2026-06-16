/**
 * `coder::tree` — recursive snapshot of a folder subtree, rendered as an
 * indented monospace tree. Wire quirks (tree.rs): the root node carries
 * NO path — the response `path` IS the root's path (never join root.name
 * onto it); child paths derive as parent + "/" + name. `non_accessible`
 * is omitted when false (the schema defaults it), `children`/`truncated`
 * are omitted when absent, and default-excluded FILES are silently
 * dropped — only excluded dirs come back as childless stub nodes.
 */
import { formatBytes } from '@/components/chat/sandbox/format'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/Tooltip'
import { iconForEntry, LockedBadge } from './entryShared'
import {
  joinEntryPath,
  safeParseRequest,
  safeParseResponse,
  type TreeNode,
  type TruncationInfo,
  treeRequestSchema,
  treeResponseSchema,
} from './parsers'

interface TreeViewProps {
  input: unknown
  output?: unknown
  running?: boolean
}

export function TreeView({ input, output, running }: TreeViewProps) {
  const req = safeParseRequest(treeRequestSchema, input)
  if (!req) return null
  const resp =
    output != null ? safeParseResponse(treeResponseSchema, output) : null
  const summary = resp ? summariseTree(resp.root) : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="path">{resp?.path ?? req.path ?? '.'}</Chip>
        {req.max_depth != null ? (
          <Chip label="depth">{req.max_depth}</Chip>
        ) : null}
        {req.per_folder_limit != null ? (
          <Chip label="limit">{req.per_folder_limit}</Chip>
        ) : null}
        {req.use_default_excludes === false ? (
          <Chip label="excludes" className="border-warn text-warn">
            off
          </Chip>
        ) : null}
        {summary ? (
          <>
            <Chip label="dirs">{summary.dirs}</Chip>
            <Chip label="files">{summary.files}</Chip>
            {summary.truncated > 0 ? (
              <Chip label="truncated" className="border-warn text-warn">
                {summary.truncated}
              </Chip>
            ) : null}
          </>
        ) : null}
        {running ? (
          <span className="font-mono text-[11px] text-ink-ghost animate-pulse">
            · scanning…
          </span>
        ) : null}
      </div>

      {resp ? (
        <div className="py-1 font-mono text-[12px] text-ink overflow-x-auto">
          <TreeRows node={resp.root} path={resp.path} depth={0} />
        </div>
      ) : (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost">
          {running ? '· scanning…' : '· no tree data'}
        </div>
      )}
    </div>
  )
}

interface TreeRowsProps {
  node: TreeNode
  /** Canonical absolute path of THIS node (root's = the response `path`). */
  path: string
  depth: number
}

/** One node row, then its children, then any truncation/empty stub. */
function TreeRows({ node, path, depth }: TreeRowsProps) {
  const isDir = node.kind === 'dir'
  const Icon = iconForEntry(node.kind, node.name)
  const children = node.children ?? []
  // `children: []` is a genuinely empty dir; absent children on a dir
  // means not descended (depth cut / exclude) and carries a stub instead.
  const isEmptyDir =
    isDir && node.children != null && children.length === 0 && !node.truncated

  return (
    <>
      <div className="flex items-center gap-1.5 px-3 py-0.5" title={path}>
        <Indent depth={depth} />
        <Icon aria-hidden className="w-3.5 h-3.5 shrink-0 text-ink-faint" />
        <span className="text-ink truncate">
          {isDir ? `${node.name}/` : node.name}
        </span>
        {node.non_accessible ? <LockedBadge /> : null}
        {!isDir ? (
          <span className="ml-auto pl-2 shrink-0 text-ink-ghost tabular-nums">
            {formatBytes(node.size)}
          </span>
        ) : null}
      </div>
      {children.map((child) => (
        <TreeRows
          key={joinEntryPath(path, child.name)}
          node={child}
          path={joinEntryPath(path, child.name)}
          depth={depth + 1}
        />
      ))}
      {node.truncated ? (
        <TruncationStub info={node.truncated} depth={depth + 1} />
      ) : null}
      {isEmptyDir ? (
        <div className="flex items-center gap-1.5 px-3 py-0.5 text-ink-ghost">
          <Indent depth={depth + 1} />· empty
        </div>
      ) : null}
    </>
  )
}

/**
 * Dimmed annotation row for a cut-off subtree. The label is the
 * at-a-glance reason; the wire's pre-written `hint` (next-step guidance)
 * renders in the tooltip.
 */
function TruncationStub({
  info,
  depth,
}: {
  info: TruncationInfo
  depth: number
}) {
  return (
    <div className="flex items-center gap-1.5 px-3 py-0.5">
      <Indent depth={depth} />
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="text-ink-ghost cursor-help">
            ⋯ {truncationLabel(info)}
          </span>
        </TooltipTrigger>
        <TooltipContent>{info.hint}</TooltipContent>
      </Tooltip>
    </div>
  )
}

/** Two spaces per level; root (depth 0) gets none. */
function Indent({ depth }: { depth: number }) {
  if (depth <= 0) return null
  return (
    <span aria-hidden className="whitespace-pre select-none">
      {'  '.repeat(depth)}
    </span>
  )
}

export interface TreeSummary {
  dirs: number
  files: number
  /** Nodes carrying a truncation stub — the at-a-glance audit signal. */
  truncated: number
}

/** Single walk over the in-memory tree for the header chips. Symlink and
    "other" nodes count in neither bucket. Includes the root itself. */
export function summariseTree(node: TreeNode): TreeSummary {
  const self: TreeSummary = {
    dirs: node.kind === 'dir' ? 1 : 0,
    files: node.kind === 'file' ? 1 : 0,
    truncated: node.truncated ? 1 : 0,
  }
  return (node.children ?? []).reduce((acc, child) => {
    const sub = summariseTree(child)
    return {
      dirs: acc.dirs + sub.dirs,
      files: acc.files + sub.files,
      truncated: acc.truncated + sub.truncated,
    }
  }, self)
}

/**
 * Stub label per truncation reason. `total` is populated ONLY for
 * "per_folder_limit" — depth cuts never peek into the folder, so they
 * have nothing to count. Unknown reasons render verbatim (the schema
 * keeps `reason` a plain string for forward tolerance).
 */
export function truncationLabel(info: TruncationInfo): string {
  switch (info.reason) {
    case 'per_folder_limit':
      return info.total != null
        ? `${info.shown}/${info.total} children shown — paginate with list-folder`
        : `${info.shown} children shown — paginate with list-folder`
    case 'max_depth':
      return 'max depth — subtree not explored'
    case 'default_exclude':
      return 'excluded by default — not descended'
    default:
      return info.reason
  }
}
