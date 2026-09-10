export interface WorkingDirectoryChangeRequest {
  sessionId: string
  path: string
}

type WorkingDirectoryChangeListener = (
  request: WorkingDirectoryChangeRequest,
) => boolean

const listeners = new Set<WorkingDirectoryChangeListener>()

export function requestWorkingDirectoryChange(
  request: WorkingDirectoryChangeRequest,
): boolean {
  const normalized = {
    sessionId: request.sessionId.trim(),
    path: request.path.trim(),
  }
  if (!normalized.sessionId || !normalized.path) return false

  for (const listener of [...listeners]) {
    if (listener(normalized)) return true
  }
  return false
}

export function onWorkingDirectoryChangeRequest(
  listener: WorkingDirectoryChangeListener,
): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
