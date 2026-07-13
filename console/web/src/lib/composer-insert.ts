/**
 * Tiny insert bus between non-chat surfaces and the active conversation's
 * composer. The Browser page publishes picked-element blocks here; the
 * composer's Lexical shell subscribes and appends the text. Inserts issued
 * while no composer is mounted (dock collapsed) are buffered and drained
 * the moment one subscribes, so a pick never disappears silently.
 */

type ComposerInsertListener = (text: string) => void

const listeners = new Set<ComposerInsertListener>()
let pending: string[] = []

export function insertIntoComposer(text: string): void {
  if (listeners.size === 0) {
    pending.push(text)
    return
  }
  for (const listener of listeners) listener(text)
}

export function onComposerInsert(listener: ComposerInsertListener): () => void {
  listeners.add(listener)
  if (pending.length > 0) {
    const drained = pending
    pending = []
    for (const text of drained) listener(text)
  }
  return () => {
    listeners.delete(listener)
  }
}
