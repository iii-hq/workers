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

type ComposerAttachListener = (files: File[]) => void

const attachListeners = new Set<ComposerAttachListener>()
let pendingFiles: File[][] = []

/** Hand files to the composer as attachments; buffered like inserts. */
export function attachToComposer(files: File[]): void {
  if (files.length === 0) return
  if (attachListeners.size === 0) {
    pendingFiles.push(files)
    return
  }
  for (const listener of attachListeners) listener(files)
}

export function onComposerAttach(listener: ComposerAttachListener): () => void {
  attachListeners.add(listener)
  if (pendingFiles.length > 0) {
    const drained = pendingFiles
    pendingFiles = []
    for (const files of drained) listener(files)
  }
  return () => {
    attachListeners.delete(listener)
  }
}

type ComposerFocusListener = () => void

const focusListeners = new Set<ComposerFocusListener>()

/** Ask the mounted composer for the caret. Unlike an insert this is not
    buffered: a focus nobody is around to take is a focus nobody wanted. */
export function requestComposerFocus(): void {
  for (const listener of focusListeners) listener()
}

export function onComposerFocusRequest(
  listener: ComposerFocusListener,
): () => void {
  focusListeners.add(listener)
  return () => {
    focusListeners.delete(listener)
  }
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
