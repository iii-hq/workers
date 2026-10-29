import { ChevronRight, Folder, FolderOpen, Plus } from 'lucide-react'
import { Hint } from '@/components/ui/Hint'
import { cn } from '@/lib/utils'

interface ConversationGroupHeaderProps {
  label: string
  /** Full project path behind a shortened label. */
  title?: string
  /** Sessions in the section, sub-agents included. */
  count: number
  collapsed: boolean
  onToggle: () => void
  /** Touch-sized header when the list is the whole narrow page. */
  narrow?: boolean
  /**
   * A project folder rather than a date bucket: drawn as the folder's own
   * row (tile, name, new-chat action) with its chats nested under it.
   */
  project?: boolean
  /** Start a chat in this project; the project header's trailing plus. */
  onCreate?: () => void
}

/**
 * A section label in the conversation tree: quiet ghost ink, the disclosure
 * caret right after the label the way a tree row carries it, and the section
 * count on the trailing edge. Reads as chrome rather than as another row — it
 * is shorter than the 28px row and never takes the selected wash.
 */
export function ConversationGroupHeader({
  label,
  title,
  count,
  collapsed,
  onToggle,
  narrow = false,
  project = false,
  onCreate,
}: ConversationGroupHeaderProps) {
  if (project) {
    return (
      <ProjectHeader
        label={label}
        title={title}
        count={count}
        collapsed={collapsed}
        onToggle={onToggle}
        narrow={narrow}
        onCreate={onCreate}
      />
    )
  }
  return (
    <button
      type="button"
      aria-expanded={!collapsed}
      title={title}
      onClick={onToggle}
      className={cn(
        'iii-ui-motion-control mt-2 flex w-full min-w-0 items-center rounded-sm border-0 bg-transparent pr-3 pl-[10px] text-left font-sans font-medium text-ink-ghost transition-colors first:mt-0 hover:bg-surface-hover hover:text-ink-faint focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ink/55',
        narrow ? 'min-h-11 text-[13px]' : 'min-h-7 text-xs',
      )}
    >
      <span className="truncate">{label}</span>
      <ChevronRight
        aria-hidden
        className={cn(
          'iii-ui-motion-control ml-0.5 size-4 shrink-0 transition-transform',
          !collapsed && 'rotate-90',
        )}
      />
      <span
        aria-hidden
        className="ml-auto pl-2 font-normal text-xs text-ink-ghost tabular-nums"
      >
        {count}
      </span>
    </button>
  )
}

/**
 * A project in the conversation tree, the way a workspace app lists repos:
 * a folder tile that opens when the project is expanded, the folder name in
 * full ink, and a plus that starts a chat already scoped to the folder. The
 * chats sit indented under it (`.conversation-project-rows`).
 */
function ProjectHeader({
  label,
  title,
  count,
  collapsed,
  onToggle,
  narrow,
  onCreate,
}: Omit<ConversationGroupHeaderProps, 'project'>) {
  const Icon = collapsed ? Folder : FolderOpen
  return (
    <div
      className={cn(
        'conversation-project-header group/project mt-3 flex w-full min-w-0 items-center gap-1 rounded-xl pr-1 first:mt-0',
        narrow ? 'min-h-12' : 'min-h-9',
      )}
    >
      <button
        type="button"
        aria-expanded={!collapsed}
        title={title}
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center gap-2.5 self-stretch rounded-xl border-0 bg-transparent pl-[7px] text-left font-inter focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ink/55"
      >
        <span
          aria-hidden
          className="flex size-6 shrink-0 items-center justify-center rounded-md bg-surface text-ink ring-1 ring-foreground/10"
        >
          <Icon className="size-4" />
        </span>
        <span
          className={cn(
            'min-w-0 truncate font-medium text-ink',
            narrow ? 'text-base' : 'text-sm',
          )}
        >
          {label}
        </span>
        <span
          aria-hidden
          className="shrink-0 text-xs text-ink-faint tabular-nums"
        >
          {count}
        </span>
      </button>
      {onCreate ? (
        <Hint label={`New chat in ${label}`} disabled={narrow}>
          <button
            type="button"
            onClick={onCreate}
            aria-label={`New chat in ${label}`}
            className={cn(
              'conversation-project-new flex shrink-0 items-center justify-center rounded-lg text-ink-faint transition-[opacity,color,background-color] duration-[var(--motion-duration-control)] hover:bg-surface-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-ink/55',
              narrow ? 'size-11' : 'size-7',
            )}
          >
            <Plus className="size-4" aria-hidden />
          </button>
        </Hint>
      ) : null}
    </div>
  )
}
