import { Ellipsis, Pencil, SquarePen, Trash2 } from 'lucide-react'
import { useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { ChatMenuContent, ChatMenuItem } from '@/components/ui/ChatMenu'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { Hint } from '@/components/ui/Hint'
import { QuietIconButton } from '@/components/ui/QuietIconButton'
import { refreshChatProjects } from '@/hooks/use-chat-projects'
import { type ChatProject, deleteProject } from '@/lib/backend/projects'

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
            render={
              <QuietIconButton
                // The header shows it with the pointer on the project; an
                // open menu keeps it (conversation-tree.css).
                data-project-menu=""
                data-menu-open={menuOpen || undefined}
                size={narrow ? 'touch' : 'default'}
              />
            }
          >
            <Ellipsis aria-hidden className="size-4" />
          </DropdownMenuTrigger>
        </Hint>
        <ChatMenuContent align="end" sideOffset={6} className="min-w-48">
          <ChatMenuItem onClick={onNewConversation}>
            <SquarePen aria-hidden />
            New conversation
          </ChatMenuItem>
          <ChatMenuItem onClick={onEdit}>
            <Pencil aria-hidden />
            Edit project…
          </ChatMenuItem>
          <DropdownMenuSeparator className="my-1.5" />
          <ChatMenuItem
            variant="destructive"
            onClick={() => setConfirming(true)}
          >
            <Trash2 aria-hidden />
            Delete project…
          </ChatMenuItem>
        </ChatMenuContent>
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
