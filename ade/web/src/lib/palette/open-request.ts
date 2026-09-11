/**
 * Anyone may ask for the palette: a worker's "Open file…" row opens it on
 * `#` so the next keystrokes already search files, the way an editor's
 * command hands over to its quick open.
 */

export interface PaletteOpenRequest {
  query?: string
}

type Listener = (request: PaletteOpenRequest) => void

const listeners = new Set<Listener>()

export function requestPaletteOpen(request: PaletteOpenRequest = {}): void {
  for (const listener of [...listeners]) listener(request)
}

export function onPaletteOpenRequest(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
