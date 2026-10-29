/**
 * Tiny insert bus between non-chat surfaces and a chat's composer. The
 * Browser page publishes picked-element blocks here, the IDE a reference to
 * the selected lines; the composer's Lexical shell subscribes and appends
 * the text. Inserts issued while no composer is mounted (dock collapsed)
 * are buffered and drained the moment one subscribes, so a pick never
 * disappears silently.
 *
 * Several chats can be open side by side, so an insert is addressed to ONE
 * composer, never all of them: the only one mounted, else the one used
 * last (`composerTarget`). A caller that must not guess asks first
 * (`mountedComposerCount`).
 */

/** Chat composers that take outside input, by id: when each was last used
 *  (0 = not yet), in the order they mounted. */
const composers = new Map<string, number>()
let usedClock = 0

/** A chat composer that takes outside input is mounted; returns its removal. */
export function registerComposer(id: string): () => void {
  composers.set(id, 0)
  return () => {
    composers.delete(id)
  }
}

/** The person put the caret in this composer: it is the one used last. */
export function markComposerUsed(id: string): void {
  if (composers.has(id)) composers.set(id, ++usedClock)
}

/** How many chat composers could take an insert right now. */
export function mountedComposerCount(): number {
  return composers.size
}

/** The composer an insert goes to: the only one, else the one used last,
 *  else the first mounted; `null` while none is. */
export function composerTarget(): string | null {
  let target: string | null = null
  let latest = -1
  for (const [id, usedAt] of composers) {
    if (usedAt > latest) {
      target = id
      latest = usedAt
    }
  }
  return target
}

/** A bus that holds what it cannot deliver yet, and drains on subscribe.
 * The buffer is bounded: with no listener the oldest entries fall off, so
 * a page publishing into a never-opened composer cannot grow memory. A
 * value addressed to one composer reaches only that composer's listeners;
 * one published while no composer is mounted goes to whoever comes next. */
const PENDING_CAP = 16

interface BusListener<T> {
  /** The composer listening; absent = every value. */
  composerId?: string
  receive: (value: T) => void
}

function bufferedBus<T>() {
  const listeners = new Set<BusListener<T>>()
  let pending: T[] = []
  return {
    publish(value: T, target: string | null = composerTarget()): void {
      const receivers = [...listeners].filter(
        (listener) =>
          target === null ||
          listener.composerId === undefined ||
          listener.composerId === target,
      )
      if (receivers.length === 0) {
        pending.push(value)
        if (pending.length > PENDING_CAP) pending.shift()
        return
      }
      for (const listener of receivers) listener.receive(value)
    },
    subscribe(receive: (value: T) => void, composerId?: string): () => void {
      const listener = { composerId, receive }
      listeners.add(listener)
      if (pending.length > 0) {
        const drained = pending
        pending = []
        for (const value of drained) receive(value)
      }
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

export interface ComposerInsert {
  text: string
  /**
   * Append at the end of the draft's last line (with a separating space)
   * instead of as a paragraph of its own — for a token such as a file
   * reference that belongs inside the sentence being written.
   */
  inline?: boolean
  /**
   * Send the draft once the text has landed, instead of leaving it for the
   * user to send. For a surface that hands over a whole prompt — a guided
   * tour's step, say — where the click WAS the decision to send.
   */
  submit?: boolean
}

type ComposerInsertListener = (insert: ComposerInsert) => void

const inserts = bufferedBus<ComposerInsert>()

export function insertIntoComposer(
  text: string,
  options: { inline?: boolean; submit?: boolean } = {},
): void {
  inserts.publish({
    text,
    inline: options.inline === true,
    submit: options.submit === true,
  })
}

/** Listen for inserts; with `composerId`, only the ones addressed to it. */
export function onComposerInsert(
  listener: ComposerInsertListener,
  composerId?: string,
): () => void {
  return inserts.subscribe(listener, composerId)
}

const draftSends = new Map<string, string>()

/**
 * Send a chat's draft as soon as its composer can (session ready, working
 * directory resolved, a model chosen) — for a surface whose click WAS the
 * decision to send, such as an example prompt in the setup wizard. Keyed by
 * chat, unlike an insert, so it reaches that chat's composer and no other;
 * `text` is the draft the decision was made for, so a draft the user has
 * edited since is theirs again and stays unsent.
 */
export function sendDraftWhenReady(conversationId: string, text: string): void {
  draftSends.set(conversationId, text)
}

/** Claim the send asked for this chat, once: the text it was asked for. */
export function claimDraftSend(conversationId: string): string | undefined {
  const text = draftSends.get(conversationId)
  draftSends.delete(conversationId)
  return text
}

type ComposerAttachListener = (files: File[]) => void

const attachments = bufferedBus<File[]>()

/** Hand files to the composer as attachments; buffered like inserts. */
export function attachToComposer(files: File[]): void {
  if (files.length === 0) return
  attachments.publish(files)
}

export function onComposerAttach(
  listener: ComposerAttachListener,
  composerId?: string,
): () => void {
  return attachments.subscribe(listener, composerId)
}

type ComposerFocusListener = () => void

const focusListeners = new Set<BusListener<void>>()

/** Ask a mounted composer for the caret: `composerId`'s, else the one an
    insert would go to. Unlike an insert this is not buffered: a focus
    nobody is around to take is a focus nobody wanted. Callable with no
    argument, so it can be handed to `requestAnimationFrame`. */
export function requestComposerFocus(composerId?: unknown): void {
  const target = typeof composerId === 'string' ? composerId : composerTarget()
  for (const listener of focusListeners) {
    if (
      target === null ||
      listener.composerId === undefined ||
      listener.composerId === target
    )
      listener.receive()
  }
}

export function onComposerFocusRequest(
  listener: ComposerFocusListener,
  composerId?: string,
): () => void {
  const entry = { composerId, receive: listener }
  focusListeners.add(entry)
  return () => {
    focusListeners.delete(entry)
  }
}
