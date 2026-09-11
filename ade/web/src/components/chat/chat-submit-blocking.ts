interface SessionHydrationState {
  realBackend: boolean
  draft?: boolean
  hydrated?: boolean
}

export function isSessionSubmitBlockedByHydration({
  realBackend,
  draft,
  hydrated,
}: SessionHydrationState): boolean {
  return realBackend && draft !== true && hydrated === false
}
