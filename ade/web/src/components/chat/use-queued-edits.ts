import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useRef,
  useState,
} from 'react'
import type { Attachment, UserMessage } from '@/types/chat'

export type QueuedEditPayload = { text: string; attachments: Attachment[] }

/** Why an edit (or an untouched original) came back to the composer. */
export type KeptReason = 'edit' | 'original'

export interface QueuedEditsOptions {
  conversationId: string
  /** `harness::unqueue`: true only when the message left the queue unsent. */
  removeQueued?: (conversationId: string, id: string) => Promise<boolean>
  /** This tab's queued drafts (read at settle time, not at render). */
  queuedDraftsRef: { readonly current: readonly UserMessage[] }
  setQueuedDrafts: Dispatch<SetStateAction<UserMessage[]>>
  /** Send a message the usual way (queued if the turn is still running). */
  send: (payload: QueuedEditPayload) => void
  /** False while the chat cannot send (no model, setup missing, switching). */
  canSend: () => boolean
  /** Put a payload into the chat's composer draft so nothing is lost. */
  keepAsDraft: (
    conversationId: string,
    payload: QueuedEditPayload,
    reason: KeptReason,
  ) => void
  /** A warning notice in the chat. */
  warn: (conversationId: string, text: string) => void
}

function sameAttachments(a: readonly Attachment[], b: readonly Attachment[]) {
  return a.length === b.length && a.every((item, i) => item.id === b[i]?.id)
}

/**
 * Editing a queued message. Browsing into one (↑) takes it back out of the
 * harness's queue (`reserve`), so a step boundary cannot deliver the
 * original while the person is still typing the edit; from then on the
 * composer holds the only copy. Submitting sends the edit (`edit`); leaving
 * it unchanged sends the original back (`release`); a composer that goes
 * away mid-edit leaves it in the chat's draft (`abandon`). An edited message
 * rejoins the queue at the end.
 *
 * `held` is every message the composer holds: browsed now, or whose edit is
 * still settling. Those are no longer queued, so the end-of-turn flush and a
 * Stop's pull-back leave them alone; they leave `held` when they settle.
 *
 * Nothing is sent twice and nothing is dropped: an edit goes out only once
 * its original is confirmed out of the queue, and an edit that cannot go out
 * (the original already reached the agent, or the chat cannot send right
 * now) lands in the composer's draft instead.
 */
export function useQueuedEdits({
  conversationId,
  removeQueued,
  queuedDraftsRef,
  setQueuedDrafts,
  send,
  canSend,
  keepAsDraft,
  warn,
}: QueuedEditsOptions) {
  const [held, setHeld] = useState<ReadonlySet<string>>(() => new Set())
  const reservationsRef = useRef(new Map<string, Promise<boolean>>())

  const hold = useCallback((id: string) => {
    setHeld((current) => {
      if (current.has(id)) return current
      const next = new Set(current)
      next.add(id)
      return next
    })
  }, [])
  const unhold = useCallback((id: string) => {
    setHeld((current) => {
      if (!current.has(id)) return current
      const next = new Set(current)
      next.delete(id)
      return next
    })
  }, [])
  const drop = useCallback(
    (id: string) =>
      setQueuedDrafts((current) => current.filter((d) => d.id !== id)),
    [setQueuedDrafts],
  )
  const take = useCallback((id: string): Promise<boolean> => {
    const reservation = reservationsRef.current.get(id)
    reservationsRef.current.delete(id)
    return reservation ?? Promise.resolve(false)
  }, [])
  const deliver = useCallback(
    (payload: QueuedEditPayload, reason: KeptReason) => {
      if (canSend()) send(payload)
      else keepAsDraft(conversationId, payload, reason)
    },
    [canSend, send, keepAsDraft, conversationId],
  )

  const reserve = useCallback(
    (id: string): Promise<boolean> => {
      if (!removeQueued) return Promise.resolve(false)
      hold(id)
      const reservation = removeQueued(conversationId, id)
        .then((removed) => {
          if (!removed) {
            warn(
              conversationId,
              'that message already reached the agent, so it cannot be edited',
            )
          }
          return removed
        })
        .catch((err) => {
          warn(
            conversationId,
            `could not take the queued message back for editing (${err instanceof Error ? err.message : String(err)}) — it will be delivered as queued`,
          )
          return false
        })
      reservationsRef.current.set(id, reservation)
      // Not taken back: it is still queued, or already went, so the
      // composer does not hold it.
      void reservation.then((removed) => {
        if (!removed) unhold(id)
      })
      return reservation
    },
    [removeQueued, conversationId, hold, unhold, warn],
  )

  // Browsing moved on without a change: the original goes back to the agent.
  const release = useCallback(
    (id: string) => {
      void take(id).then((reserved) => {
        if (reserved) {
          const draft = queuedDraftsRef.current.find((d) => d.id === id)
          drop(id)
          if (draft) {
            deliver(
              { text: draft.content, attachments: draft.attachments ?? [] },
              'original',
            )
          }
        }
        unhold(id)
      })
    },
    [take, queuedDraftsRef, drop, deliver, unhold],
  )

  // Enter on a browsed message: send the edit (`payload`), or remove the
  // message (`null`, the composer was emptied).
  const edit = useCallback(
    (id: string, payload: QueuedEditPayload | null) => {
      void take(id).then(async (reserved) => {
        // Enter before the reservation landed, or a reservation that errored:
        // the edit may go only once the original is confirmed out.
        const removed =
          reserved ||
          (removeQueued
            ? await removeQueued(conversationId, id).catch(() => false)
            : false)
        const original = queuedDraftsRef.current.find((d) => d.id === id)
        if (removed) drop(id)
        unhold(id)
        if (payload === null) return
        if (removed) {
          deliver(payload, 'edit')
          return
        }
        // The original reached the agent (or is still queued); sending the
        // edit too would deliver the message twice. Unless nothing changed,
        // the edit waits in the composer for the person to decide.
        const unchanged =
          original !== undefined &&
          original.content.trim() === payload.text &&
          sameAttachments(original.attachments ?? [], payload.attachments)
        if (!unchanged) keepAsDraft(conversationId, payload, 'edit')
      })
    },
    [
      take,
      removeQueued,
      conversationId,
      queuedDraftsRef,
      drop,
      unhold,
      deliver,
      keepAsDraft,
    ],
  )

  // The composer went away (another chat, a remount) mid-edit. If the
  // message was taken out of the queue, the editor held its only copy, so it
  // moves into the chat's draft rather than vanish.
  const abandon = useCallback(
    (id: string, payload: QueuedEditPayload) => {
      void take(id).then((reserved) => {
        if (reserved) {
          const original = queuedDraftsRef.current.find((d) => d.id === id)
          drop(id)
          // An emptied editor is not a decision to remove (that takes
          // Enter), so the original is what comes back.
          const empty =
            payload.text.trim().length === 0 && payload.attachments.length === 0
          const kept =
            empty && original
              ? {
                  text: original.content,
                  attachments: original.attachments ?? [],
                }
              : payload
          if (kept.text.trim() || kept.attachments.length > 0) {
            keepAsDraft(conversationId, kept, empty ? 'original' : 'edit')
          }
        }
        unhold(id)
      })
    },
    [take, queuedDraftsRef, drop, keepAsDraft, conversationId, unhold],
  )

  return { held, reserve, release, edit, abandon }
}
