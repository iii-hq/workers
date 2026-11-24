/**
 * Anyone may ask for the Models dialog: ⌘K's provider rows open it on a
 * provider's settings, on the Add a provider page, or with its search filled
 * in so a provider not yet installed shows with its Add provider action.
 * `ModelLibraryHost` answers.
 */

export type ModelLibraryRequest =
  | { view: 'library'; query?: string }
  | { view: 'provider'; providerId: string }
  | { view: 'add-provider' }

type Listener = (request: ModelLibraryRequest) => void

const listeners = new Set<Listener>()

export function requestModelLibrary(
  request: ModelLibraryRequest = { view: 'library' },
): void {
  for (const listener of [...listeners]) listener(request)
}

export function onModelLibraryRequest(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
