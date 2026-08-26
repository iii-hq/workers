export function workingDirectoryScopeMismatch(
  root: string | null,
  workingDir: string | null | undefined,
  conversationId: string | null | undefined,
  canRequestChange: boolean,
): boolean {
  return !!(
    canRequestChange &&
    conversationId &&
    root &&
    root !== (workingDir ?? null)
  )
}

export function workingDirectoryScopeMessage(
  root: string,
  workingDir: string | null | undefined,
): string {
  return workingDir
    ? `Browsing ${root}; chat still works in ${workingDir}.`
    : `Browsing ${root}; chat has no working directory.`
}
