/**
 * Ephemeral link from the traces screen to the chat: "go to message".
 *
 * A tiny latest-event store modeled on `panel-context.ts`: the request is
 * stored BEFORE listeners run, so a ChatView mounted in response to it reads
 * the event on first render. The consumer clears the event after acting on
 * it — a later remount of the same conversation must never re-apply a stale
 * request.
 *
 * The identity this direction relies on is stamped as baggage on every span
 * of a harness turn (`harness/src/functions/turn.rs`): `iii.session.id` is
 * the engine session_id (= `Conversation.id`) and `iii.message.id` is the
 * harness turn id (`t_<uuid>`) — NOT the console's `msg-<uuid>` message id.
 *
 * (The chat → traces direction needs no store: the traces screen follows the
 * active conversation automatically — see the session scope in
 * `pages/TracesV2/index.tsx`.)
 */

import { useSyncExternalStore } from 'react'

/** Traces → chat: land the transcript on one turn's messages. */
export interface ChatMessageFocusEvent {
  id: number
  /** engine session_id (= `Conversation.id`) */
  sessionId: string
  /** harness turn id (`t_<uuid>`) — the spans' `iii.message.id` */
  turnId: string
}

let nextEventId = 1
let chatMessageFocus: ChatMessageFocusEvent | undefined
const chatMessageFocusListeners = new Set<() => void>()

function emit(): void {
  for (const listener of [...chatMessageFocusListeners]) listener()
}

function subscribeChatMessageFocus(listener: () => void): () => void {
  chatMessageFocusListeners.add(listener)
  return () => chatMessageFocusListeners.delete(listener)
}

/** Ask the chat to land on one turn's messages once its session is open. */
export function requestChatMessageFocus(request: {
  sessionId: string
  turnId: string
}): ChatMessageFocusEvent {
  chatMessageFocus = {
    id: nextEventId++,
    sessionId: request.sessionId,
    turnId: request.turnId,
  }
  emit()
  return chatMessageFocus
}

export function getChatMessageFocus(): ChatMessageFocusEvent | undefined {
  return chatMessageFocus
}

/** Reactive pending turn focus for the mounted chat view. */
export function useChatMessageFocus(): ChatMessageFocusEvent | undefined {
  return useSyncExternalStore(
    subscribeChatMessageFocus,
    () => chatMessageFocus,
    () => undefined,
  )
}

/** Consume a handled turn focus (id-guarded against a newer event). */
export function clearChatMessageFocus(id: number): void {
  if (chatMessageFocus?.id !== id) return
  chatMessageFocus = undefined
  emit()
}

/**
 * Completion writes the turn's last transcript rows AFTER the session's
 * status flips idle — dropping on the flip itself would eat a landing whose
 * anchor is milliseconds away. Consumers wait this long in the droppable
 * state before actually clearing the request.
 */
export const CHAT_FOCUS_DROP_GRACE_MS = 2_000

/**
 * Whether a pending focus request is provably dead: the transcript is
 * hydrated, no anchor row resolved, and no turn is running that could still
 * write it. A working session holds the request instead — a live turn
 * persists its durable rows as it goes, and landing when they appear is
 * still what the click asked for. `hydrated: undefined` counts as hydrated
 * (backends that never track hydration).
 */
export function shouldDropChatFocus(state: {
  hydrated?: boolean
  working: boolean
  anchored: boolean
}): boolean {
  return !state.anchored && state.hydrated !== false && !state.working
}

/** Test-only reset; exported to keep the store deterministic in unit tests. */
export function resetTraceLinksForTests(): void {
  nextEventId = 1
  chatMessageFocus = undefined
  chatMessageFocusListeners.clear()
}
