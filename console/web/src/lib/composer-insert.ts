/**
 * Tiny insert bus between non-chat surfaces and the active conversation's
 * composer. The Browser page publishes picked-element blocks here; the
 * composer's Lexical shell subscribes and appends the text. Inserts issued
 * while no composer is mounted (dock collapsed) are buffered and drained
 * the moment one subscribes, so a pick never disappears silently.
 */

/** A bus that holds what it cannot deliver yet, and drains on subscribe.
 * The buffer is bounded: with no listener the oldest entries fall off, so
 * a page publishing into a never-opened composer cannot grow memory. */
const PENDING_CAP = 16

function bufferedBus<T>() {
  const listeners = new Set<(value: T) => void>()
  let pending: T[] = []
  return {
    publish(value: T): void {
      if (listeners.size === 0) {
        pending.push(value)
        if (pending.length > PENDING_CAP) pending.shift()
        return
      }
      for (const listener of listeners) listener(value)
    },
    subscribe(listener: (value: T) => void): () => void {
      listeners.add(listener)
      if (pending.length > 0) {
        const drained = pending
        pending = []
        for (const value of drained) listener(value)
      }
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

type ComposerInsertListener = (text: string) => void

const inserts = bufferedBus<string>()

export function insertIntoComposer(text: string): void {
  inserts.publish(text)
}

export function onComposerInsert(listener: ComposerInsertListener): () => void {
  return inserts.subscribe(listener)
}

type ComposerAttachListener = (files: File[]) => void

const attachments = bufferedBus<File[]>()

/** Hand files to the composer as attachments; buffered like inserts. */
export function attachToComposer(files: File[]): void {
  if (files.length === 0) return
  attachments.publish(files)
}

export function onComposerAttach(listener: ComposerAttachListener): () => void {
  return attachments.subscribe(listener)
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
