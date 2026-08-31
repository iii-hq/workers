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
    ? `Browsing ${root}. Chat still works in ${workingDir}.`
    : `Browsing ${root}. Chat has no working directory yet.`
}
