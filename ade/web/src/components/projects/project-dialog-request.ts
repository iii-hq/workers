/**
 * Opening "New project" / "Edit project" from anywhere (the sidebar, the
 * chat header's project chip, the palette): one host renders the dialog, the
 * most recently mounted one, so two chat panes never open two dialogs.
 */
export type ProjectDialogRequest =
  | { mode: 'new'; folders?: readonly string[] }
  | { mode: 'edit'; projectId: string }

type Listener = (request: ProjectDialogRequest) => void

const hosts: Listener[] = []

export function requestProjectDialog(request: ProjectDialogRequest): boolean {
  const host = hosts.at(-1)
  if (!host) return false
  host(request)
  return true
}

export function onProjectDialogRequest(listener: Listener): () => void {
  hosts.push(listener)
  return () => {
    const index = hosts.lastIndexOf(listener)
    if (index !== -1) hosts.splice(index, 1)
  }
}
