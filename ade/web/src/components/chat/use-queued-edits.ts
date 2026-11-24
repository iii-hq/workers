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
  /**
   * `harness::status` → `queued`: used to settle an unqueue whose answer was
   * lost (the call rejected, but the removal may have committed).
   */
  listQueued?: (conversationId: string) => Promise<readonly { id: string }[]>
  /** This tab's queued drafts (read at settle time, not at render). */
  queuedDraftsRef: { readonly current: readonly UserMessage[] }
  setQueuedDrafts: Dispatch<SetStateAction<UserMessage[]>>
  /**
   * Send a message the usual way (queued if the turn is still running).
   * `reason` says whether it is the person's edit or the untouched original.
   */
  send: (payload: QueuedEditPayload, reason: KeptReason) => void
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

function samePayload(a: QueuedEditPayload, b: QueuedEditPayload) {
  return (
    a.text.trim() === b.text.trim() &&
    sameAttachments(a.attachments, b.attachments)
  )
}

/**
 * Where an unqueue left the message: `removed` (out of the queue, the
 * composer holds the only copy), `queued` (still queued or already
 * delivered, so the composer must not send it), or `unknown` (the call
 * failed and the queue could not prove the message is still there; it may
 * have been removed with the answer lost, so it is kept, never resent).
 */
type Outcome = 'removed' | 'queued' | 'unknown'

/** One browse's claim on a message: only the newest may let go of it. */
interface Hold {
  token: number
  /** The Stop count when the hold began; a Stop since means no auto-send. */
  stops: number
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
 * Each browse owns its hold by token, so an older attempt settling late
 * never lets go of a newer one's hold.
 *
 * Nothing is sent twice and nothing is dropped: an edit goes out only once
 * its original is confirmed out of the queue, and an edit that cannot go out
 * (the original already reached the agent, the chat cannot send right now,
 * or a Stop came in between) lands in the composer's draft instead.
 */
export function useQueuedEdits({
  conversationId,
  removeQueued,
  listQueued,
  queuedDraftsRef,
  setQueuedDrafts,
  send,
  canSend,
  keepAsDraft,
  warn,
}: QueuedEditsOptions) {
  const [held, setHeld] = useState<ReadonlySet<string>>(() => new Set())
  const holdsRef = useRef(new Map<string, Hold>())
  const nextTokenRef = useRef(0)
  const stopsRef = useRef(0)
  const reservationsRef = useRef(new Map<string, Promise<Outcome>>())
  // What each browsed message said when browsing began, so an edit can be
  // told apart from the original after the queue row has gone.
  const originalsRef = useRef(new Map<string, QueuedEditPayload>())
  // A release still waiting for its reservation to answer. Browsing back
  // into the message before it does adopts the reservation, and the release
  // stands down (see `reserve` and `release`).
  const releasingRef = useRef(
    new Map<string, { reservation: Promise<Outcome> }>(),
  )

  const hold = useCallback((id: string): Hold => {
    const claim = { token: ++nextTokenRef.current, stops: stopsRef.current }
    holdsRef.current.set(id, claim)
    setHeld((current) => {
      if (current.has(id)) return current
      const next = new Set(current)
      next.add(id)
      return next
    })
    return claim
  }, [])
  // Only the hold's owner lets go; a stale attempt is a no-op.
  const unhold = useCallback((id: string, claim: Hold | undefined) => {
    if (!claim || holdsRef.current.get(id) !== claim) return
    holdsRef.current.delete(id)
    originalsRef.current.delete(id)
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
  const take = useCallback((id: string): Promise<Outcome | null> => {
    const reservation = reservationsRef.current.get(id)
    reservationsRef.current.delete(id)
    return reservation ?? Promise.resolve(null)
  }, [])
  const originalOf = useCallback(
    (id: string): QueuedEditPayload | undefined => {
      const draft = queuedDraftsRef.current.find((d) => d.id === id)
      if (draft) {
        return { text: draft.content, attachments: draft.attachments ?? [] }
      }
      return originalsRef.current.get(id)
    },
    [queuedDraftsRef],
  )
  const deliver = useCallback(
    (payload: QueuedEditPayload, reason: KeptReason, claim?: Hold) => {
      // A Stop since browsing began: the person asked for nothing more to
      // go out on its own, so the original waits in the composer instead.
      const stoppedSince =
        reason === 'original' && claim && claim.stops !== stopsRef.current
      if (!stoppedSince && canSend()) send(payload, reason)
      else keepAsDraft(conversationId, payload, reason)
    },
    [canSend, send, keepAsDraft, conversationId],
  )

  // `harness::unqueue`, with a failed call settled against the queue itself:
  // only a queue that still lists the message proves it was not removed.
  const unqueue = useCallback(
    async (id: string): Promise<Outcome> => {
      if (!removeQueued) return 'queued'
      try {
        return (await removeQueued(conversationId, id)) ? 'removed' : 'queued'
      } catch {
        if (!listQueued) return 'unknown'
        try {
          const rows = await listQueued(conversationId)
          return rows.some((row) => row.id === id) ? 'queued' : 'unknown'
        } catch {
          return 'unknown'
        }
      }
    },
    [removeQueued, listQueued, conversationId],
  )

  const reserve = useCallback(
    (id: string): Promise<boolean> => {
      if (!removeQueued) return Promise.resolve(false)
      const claim = hold(id)
      if (!originalsRef.current.has(id)) {
        const original = originalOf(id)
        if (original) originalsRef.current.set(id, original)
      }
      // One request per message. Browsing back in while the first is still
      // out (↑, ↓, ↑ faster than the harness answers) adopts it: the
      // release that was waiting on it stands down, and no second request
      // goes out to be told the message is "already gone" and clear the
      // editor the person has just re-entered.
      const releasing = releasingRef.current.get(id)
      releasingRef.current.delete(id)
      const existing = reservationsRef.current.get(id) ?? releasing?.reservation
      const reservation =
        existing ??
        unqueue(id).then((outcome) => {
          if (outcome === 'queued') {
            warn(
              conversationId,
              'that message already reached the agent, so it cannot be edited',
            )
          } else if (outcome === 'unknown') {
            warn(
              conversationId,
              'could not confirm the queued message was taken back for editing — it stays in the composer and will not be sent on its own',
            )
          }
          return outcome
        })
      reservationsRef.current.set(id, reservation)
      // Not taken back: it is still queued, or already went, so the
      // composer does not hold it. An unclear answer stays with the
      // composer, which never resends it unasked.
      return reservation.then((outcome) => {
        if (outcome === 'queued') unhold(id, claim)
        return outcome !== 'queued'
      })
    },
    [removeQueued, conversationId, hold, unhold, warn, unqueue, originalOf],
  )

  // Browsing moved on without a change: the original goes back to the agent.
  const release = useCallback(
    (id: string) => {
      const claim = holdsRef.current.get(id)
      const reservation = reservationsRef.current.get(id)
      reservationsRef.current.delete(id)
      if (!reservation) {
        unhold(id, claim)
        return
      }
      const releasing = { reservation }
      releasingRef.current.set(id, releasing)
      void reservation.then((outcome) => {
        // Browsed back in before the answer came: the new browse owns the
        // reservation, and the message stays where it is.
        if (releasingRef.current.get(id) !== releasing) return
        releasingRef.current.delete(id)
        if (outcome !== 'queued') {
          const original = originalOf(id)
          drop(id)
          if (original) {
            // An unclear unqueue may have left the original queued: it goes
            // to the draft, never out a second time.
            if (outcome === 'removed') deliver(original, 'original', claim)
            else keepAsDraft(conversationId, original, 'original')
          }
        }
        unhold(id, claim)
      })
    },
    [originalOf, drop, deliver, keepAsDraft, conversationId, unhold],
  )

  // Enter on a browsed message: send the edit (`payload`), or remove the
  // message (`null`, the composer was emptied).
  const edit = useCallback(
    (id: string, payload: QueuedEditPayload | null) => {
      const claim = holdsRef.current.get(id)
      void take(id).then(async (reserved) => {
        // Enter before the reservation landed, or a reservation that said
        // no: the edit may go only once the original is confirmed out.
        const outcome =
          reserved === 'removed' || reserved === 'unknown'
            ? reserved
            : await unqueue(id)
        const original = originalOf(id)
        if (outcome !== 'queued') drop(id)
        unhold(id, claim)
        if (payload === null) return
        if (outcome === 'removed') {
          deliver(payload, 'edit', claim)
          return
        }
        if (outcome === 'unknown') {
          // The original may still be queued: the edit waits for the
          // person rather than risk a second delivery.
          keepAsDraft(conversationId, payload, 'edit')
          return
        }
        // The original reached the agent (or is still queued); sending the
        // edit too would deliver the message twice. Unless nothing changed,
        // the edit waits in the composer for the person to decide.
        const unchanged =
          original !== undefined && samePayload(original, payload)
        if (!unchanged) keepAsDraft(conversationId, payload, 'edit')
      })
    },
    [
      take,
      unqueue,
      conversationId,
      originalOf,
      drop,
      unhold,
      deliver,
      keepAsDraft,
    ],
  )

  // The composer went away (another chat, a remount) mid-edit. If the
  // message was taken out of the queue, the editor held its only copy, so it
  // moves into the chat's draft rather than vanish. If it was never taken
  // back, the original is delivered as queued, and only a changed edit is
  // kept (as a draft, the original is not queued again).
  const abandon = useCallback(
    (id: string, payload: QueuedEditPayload) => {
      const claim = holdsRef.current.get(id)
      const original = originalOf(id)
      void take(id).then((reserved) => {
        const empty =
          payload.text.trim().length === 0 && payload.attachments.length === 0
        if (reserved === 'removed' || reserved === 'unknown') {
          drop(id)
          // An emptied editor is not a decision to remove (that takes
          // Enter), so the original is what comes back.
          const kept = empty && original ? original : payload
          if (kept.text.trim() || kept.attachments.length > 0) {
            keepAsDraft(conversationId, kept, empty ? 'original' : 'edit')
          }
        } else if (
          !empty &&
          original !== undefined &&
          !samePayload(original, payload)
        ) {
          keepAsDraft(conversationId, payload, 'edit')
        }
        unhold(id, claim)
      })
    },
    [take, originalOf, drop, keepAsDraft, conversationId, unhold],
  )

  // A Stop came in: a release that settles after it puts the original in
  // the draft instead of starting another turn.
  const stopped = useCallback(() => {
    stopsRef.current += 1
  }, [])

  return { held, reserve, release, edit, abandon, stopped }
}
