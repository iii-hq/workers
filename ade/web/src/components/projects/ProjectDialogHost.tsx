import { useEffect, useState } from 'react'
import type { ChatProject } from '@/lib/backend/projects'
import type { Conversation } from '@/types/chat'
import { ProjectDialog } from './ProjectDialog'
import {
  onProjectDialogRequest,
  type ProjectDialogRequest,
} from './project-dialog-request'

/**
 * The one place a chat pane renders the project dialog: it answers
 * `requestProjectDialog` from the sidebar, the chat header's project chip and
 * the settings page.
 */
export function ProjectDialogHost({
  projects,
  conversations,
}: {
  projects: readonly ChatProject[]
  conversations: readonly Conversation[]
}) {
  const [request, setRequest] = useState<ProjectDialogRequest | null>(null)
  const [open, setOpen] = useState(false)
  useEffect(
    () =>
      onProjectDialogRequest((next) => {
        setRequest(next)
        setOpen(true)
      }),
    [],
  )
  const project =
    request?.mode === 'edit'
      ? (projects.find((p) => p.id === request.projectId) ?? null)
      : null
  return (
    <ProjectDialog
      open={open && (request?.mode === 'new' || project !== null)}
      onOpenChange={setOpen}
      project={project}
      projects={projects}
      conversations={conversations}
      initialFolders={request?.mode === 'new' ? request.folders : undefined}
      initialSection={request?.mode === 'edit' ? request.section : undefined}
    />
  )
}
