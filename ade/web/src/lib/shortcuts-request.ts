/**
 * Anyone may ask for the keyboard-shortcuts dialog: the sidebar's help
 * button, a palette row, a worker page. `App` owns the dialog and listens.
 */

type Listener = () => void

const listeners = new Set<Listener>()

export function requestShortcutsDialog(): void {
  for (const listener of [...listeners]) listener()
}

export function onShortcutsDialogRequest(listener: Listener): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}
