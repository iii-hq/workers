import { ChevronRight, Folder, FolderOpen, Plus } from 'lucide-react'
import type { ReactNode } from 'react'
import { ProjectDot } from '@/components/projects/ProjectDot'
import {
  DropdownMenu,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { ChatMenuContent, ChatMenuItem } from '@/components/ui/ChatMenu'
import { Hint } from '@/components/ui/Hint'
import { QuietIconButton } from '@/components/ui/QuietIconButton'
import { type ChatProject, tildePath } from '@/lib/backend/projects'
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
  /**
   * A chat project (chat-projects §1.3.1): drawn with its colour dot, the
   * folder count (the conversation count on a phone), the "+" and its menu.
   */
  chatProject?: ChatProject
  /** Start a chat in one of the chat project's folders. */
  onCreateIn?: (folder: string) => void
  /** The chat project's "⋯" menu. */
  menu?: ReactNode
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
  chatProject,
  onCreateIn,
  menu,
}: ConversationGroupHeaderProps) {
  if (chatProject) {
    return (
      <ChatProjectHeader
        project={chatProject}
        count={count}
        collapsed={collapsed}
        onToggle={onToggle}
        narrow={narrow}
        onCreateIn={onCreateIn}
        menu={menu}
      />
    )
  }
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
        'conversation-group-header iii-ui-motion-control mt-2 flex w-full min-w-0 items-center rounded-sm border-0 bg-transparent pr-3 pl-2.5 text-left font-inter font-medium text-ink-ghost transition-colors hover:bg-surface-hover hover:text-ink-faint focus-visible:ring-3 focus-visible:outline-none focus-visible:outline-offset-1 focus-visible:ring-ring/50',
        narrow ? 'min-h-11 text-sm' : 'min-h-8 text-sm',
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
 * A chat project's header (chat-projects §1.3.1): an 8px colour dot, the
 * name, then "3 folders" (a phone shows the conversation count instead), a
 * "+" for a new conversation in the project — on hover with a mouse, always
 * on touch — and the "⋯" menu. A tap folds or unfolds the project. With
 * several folders the "+" asks which one.
 */
function ChatProjectHeader({
  project,
  count,
  collapsed,
  onToggle,
  narrow,
  onCreateIn,
  menu,
}: {
  project: ChatProject
  count: number
  collapsed: boolean
  onToggle: () => void
  narrow: boolean
  onCreateIn?: (folder: string) => void
  menu?: ReactNode
}) {
  const folders = project.folders.length
  // The header shows it with the pointer on the project (conversation-tree.css).
  const plus = (
    <QuietIconButton data-project-new="" size={narrow ? 'touch' : 'default'} />
  )
  const plusLabel = `New conversation in ${project.name}`
  return (
    <div
      data-chat-project={project.id}
      className={cn(
        'conversation-project-header group/project mt-3 flex w-full min-w-0 items-center gap-0.5 rounded-sm pr-1',
        narrow ? 'min-h-12' : 'min-h-9',
      )}
    >
      <button
        type="button"
        aria-expanded={!collapsed}
        title={project.folders.map(tildePath).join('\n')}
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center gap-2.5 self-stretch rounded-sm border-0 bg-transparent pl-3 text-left font-inter focus-visible:ring-3 focus-visible:outline-none focus-visible:outline-offset-1 focus-visible:ring-ring/50"
      >
        <ProjectDot color={project.color} />
        <span
          className={cn(
            'min-w-0 truncate font-medium text-ink',
            narrow ? 'text-base' : 'text-sm',
          )}
        >
          {project.name}
        </span>
        <span className="shrink-0 text-xs text-ink-faint tabular-nums">
          {narrow
            ? count
            : `${folders} ${folders === 1 ? 'folder' : 'folders'}`}
        </span>
        <ChevronRight
          aria-hidden
          className={cn(
            'iii-ui-motion-control size-4 shrink-0 text-ink-ghost transition-transform',
            !collapsed && 'rotate-90',
          )}
        />
      </button>
      {onCreateIn && folders === 1 ? (
        <Hint label={plusLabel} disabled={narrow}>
          <QuietIconButton
            data-project-new=""
            size={narrow ? 'touch' : 'default'}
            aria-label={plusLabel}
            onClick={() => onCreateIn(project.folders[0])}
          >
            <Plus className="size-4" aria-hidden />
          </QuietIconButton>
        </Hint>
      ) : onCreateIn && folders > 1 ? (
        <DropdownMenu>
          <Hint label={plusLabel} disabled={narrow}>
            <DropdownMenuTrigger aria-label={plusLabel} render={plus}>
              <Plus className="size-4" aria-hidden />
            </DropdownMenuTrigger>
          </Hint>
          <ChatMenuContent align="end" sideOffset={6} className="min-w-56">
            <DropdownMenuGroup>
              <DropdownMenuLabel>Choose a folder</DropdownMenuLabel>
              {project.folders.map((folder) => (
                <ChatMenuItem key={folder} onClick={() => onCreateIn(folder)}>
                  <Folder aria-hidden className="text-ink-faint" />
                  <span
                    data-keep-mono=""
                    className="truncate font-mono text-xs"
                  >
                    {tildePath(folder)}
                  </span>
                </ChatMenuItem>
              ))}
            </DropdownMenuGroup>
          </ChatMenuContent>
        </DropdownMenu>
      ) : null}
      {menu}
    </div>
  )
}

/**
 * A project in the conversation tree, the way a workspace app lists repos:
 * a quiet outline folder (open while its chats show), the folder name in
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
  const ProjectIcon = collapsed ? Folder : FolderOpen
  return (
    <div
      className={cn(
        'conversation-project-header group/project mt-3 flex w-full min-w-0 items-center gap-1 rounded-sm pr-1',
        narrow ? 'min-h-12' : 'min-h-9',
      )}
    >
      <button
        type="button"
        aria-expanded={!collapsed}
        title={title}
        onClick={onToggle}
        className="flex min-w-0 flex-1 items-center gap-2.5 self-stretch rounded-sm border-0 bg-transparent pl-2.5 text-left font-inter focus-visible:ring-3 focus-visible:outline-none focus-visible:outline-offset-1 focus-visible:ring-ring/50"
      >
        <ProjectIcon
          aria-hidden
          className="size-4 shrink-0 text-ink-faint"
          data-project-icon=""
        />
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
          <QuietIconButton
            data-project-new=""
            size={narrow ? 'touch' : 'default'}
            onClick={onCreate}
            aria-label={`New chat in ${label}`}
          >
            <Plus className="size-4" aria-hidden />
          </QuietIconButton>
        </Hint>
      ) : null}
    </div>
  )
}
