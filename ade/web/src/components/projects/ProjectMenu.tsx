import { Ellipsis, Pencil, SquarePen, Trash2 } from 'lucide-react'
import { useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { Hint } from '@/components/ui/Hint'
import { refreshChatProjects } from '@/hooks/use-chat-projects'
import { type ChatProject, deleteProject } from '@/lib/backend/projects'
import { cn } from '@/lib/utils'

const MENU_ITEM_CLASS = 'gap-2.5 rounded-lg px-2.5 py-2 text-sm'

/** "Its 4 conversations stay, under No project." */
export function deleteProjectText(count: number): string {
  if (count === 0) return 'It has no conversations.'
  if (count === 1) return 'Its 1 conversation stays, under No project.'
  return `Its ${count} conversations stay, under No project.`
}

/**
 * The project's menu (chat-projects §1.3.2): "New conversation", "Edit
 * project…", "Delete project…". Deleting confirms first; the conversations
 * stay, under No project.
 */
export function ProjectMenu({
  project,
  conversationCount,
  onNewConversation,
  onEdit,
  narrow = false,
  onDeleted,
  remove = deleteProject,
}: {
  project: Pick<ChatProject, 'id' | 'name'>
  /** Conversations in the project, for the confirmation's text. */
  conversationCount: number
  onNewConversation: () => void
  onEdit: () => void
  narrow?: boolean
  onDeleted?: () => void
  /** The engine call; tests replace it. */
  remove?: (id: string) => Promise<unknown>
}) {
  const [menuOpen, setMenuOpen] = useState(false)
  const [confirming, setConfirming] = useState(false)
  return (
    <>
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <Hint label="More" disabled={narrow}>
          <DropdownMenuTrigger
            aria-label={`more actions for project ${project.name}`}
            data-menu-open={menuOpen || undefined}
            className={cn(
              'conversation-project-menu flex shrink-0 items-center justify-center rounded-sm text-ink-faint transition-[opacity,color,background-color] duration-[var(--motion-duration-control)] hover:bg-surface-hover hover:text-ink focus-visible:ring-3 focus-visible:outline-none focus-visible:ring-ring/50',
              narrow ? 'size-11' : 'size-7',
            )}
          >
            <Ellipsis aria-hidden className="size-4" />
          </DropdownMenuTrigger>
        </Hint>
        <DropdownMenuContent
          align="end"
          sideOffset={6}
          className="min-w-48 p-1.5"
        >
          <DropdownMenuItem
            className={MENU_ITEM_CLASS}
            onClick={onNewConversation}
          >
            <SquarePen aria-hidden />
            New conversation
          </DropdownMenuItem>
          <DropdownMenuItem className={MENU_ITEM_CLASS} onClick={onEdit}>
            <Pencil aria-hidden />
            Edit project…
          </DropdownMenuItem>
          <DropdownMenuSeparator className="my-1.5" />
          <DropdownMenuItem
            variant="destructive"
            className={MENU_ITEM_CLASS}
            onClick={() => setConfirming(true)}
          >
            <Trash2 aria-hidden />
            Delete project…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={`Delete “${project.name}”?`}
        description={deleteProjectText(conversationCount)}
        confirmLabel="Delete"
        busyLabel="Deleting…"
        tone="danger"
        onConfirm={async () => {
          // A rejection stays in the dialog, under the description.
          await remove(project.id)
          void refreshChatProjects()
          onDeleted?.()
        }}
      />
    </>
  )
}
