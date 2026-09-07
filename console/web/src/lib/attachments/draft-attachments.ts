/**
 * Composer attachment chips that survive leaving the conversation.
 *
 * The composer's text already outlives a session switch: `use-conversations`
 * keeps it per conversation and, for server-backed sessions, parks it through
 * `session::set-draft`. The chips did not — they lived in the composer's
 * local state, and ChatView remounts the composer per conversation, so
 * opening another chat and coming back lost every attachment.
 *
 * This module is the pure half of the fix. The hook owns the timing (debounce,
 * upload chain, which conversation is local); everything here is a decision
 * about lists of chips, so it can be tested without React or a network:
 *
 * - `createDraftAttachmentStore` — the per-conversation in-memory list the
 *   composer is re-seeded from, mirroring the text's ref map.
 * - `draftAttachmentsFromMeta` — `SessionMeta.draft_attachments` → chips.
 * - `releaseRemovedDraftAttachments` — which stored chips a change orphaned.
 * - `mergeSyncedAttachments` — fold what the hook learned about a chip (its
 *   server id, its rebuilt bytes) back into the composer's own list.
 */

import { deleteAttachment } from '@/lib/sessions/api'
import type { SessionMeta } from '@/lib/sessions/types'
import type { Attachment } from '@/types/chat'

/**
 * Why the composer's chip list changed. Only a `remove` may release stored
 * bytes: a `submit` or `edit` clears chips that a message now references, and
 * `hydrate` only adds bytes to chips already known on both sides.
 */
export type DraftAttachmentChangeReason =
  | 'attach'
  | 'remove'
  | 'submit'
  | 'edit'
  | 'hydrate'

export interface DraftAttachmentChange {
  reason: DraftAttachmentChangeReason
}

/** One conversation's live chips, keyed by conversation id. */
export interface DraftAttachmentStore {
  get(id: string): Attachment[] | undefined
  /**
   * Record the composer's list. Fields the store already learned for a chip
   * (`attachmentId` from an upload, `file`/`dataUrl` from hydration) are
   * carried onto the incoming version when it lacks them: the composer
   * reports its own state, which can lag behind what the store knows.
   * Returns the list actually kept.
   */
  set(id: string, attachments: Attachment[]): Attachment[]
  /** Patch one chip in place; `undefined` when the chip is no longer there. */
  patch(
    id: string,
    chipId: string,
    patch: Partial<Pick<Attachment, 'attachmentId' | 'file' | 'dataUrl'>>,
  ): Attachment[] | undefined
  delete(id: string): void
}

export function createDraftAttachmentStore(): DraftAttachmentStore {
  const lists = new Map<string, Attachment[]>()
  return {
    get: (id) => lists.get(id),
    set: (id, attachments) => {
      const merged = mergeSyncedAttachments(attachments, lists.get(id) ?? [])
      lists.set(id, merged)
      return merged
    },
    patch: (id, chipId, patch) => {
      const current = lists.get(id)
      if (!current?.some((a) => a.id === chipId)) return undefined
      const next = current.map((a) =>
        a.id === chipId ? { ...a, ...patch } : a,
      )
      lists.set(id, next)
      return next
    },
    delete: (id) => {
      lists.delete(id)
    },
  }
}

/**
 * Chips for the attachments a session has parked with its draft. The server
 * id doubles as the chip id — nothing local ever named these — and there are
 * no bytes yet; ChatView fetches them in the background so the chip can be
 * expanded on send like a fresh pick.
 */
export function draftAttachmentsFromMeta(
  meta: Pick<SessionMeta, 'draft_attachments'>,
): Attachment[] | undefined {
  const parked = meta.draft_attachments
  if (!parked || parked.length === 0) return undefined
  return parked.map((a) => ({
    id: a.attachment_id,
    name: a.name,
    size: a.size,
    type: a.mime,
    attachmentId: a.attachment_id,
  }))
}

/**
 * A fresh `SessionMeta` names the parked attachments again; keep the chip
 * objects this tab already built for them (their local id, bytes and
 * thumbnail) so a directory refresh does not turn a hydrated chip back into
 * a bare one. Membership and order follow the server.
 */
export function reconcileDraftAttachments(
  existing: Attachment[] | undefined,
  fromMeta: Attachment[] | undefined,
): Attachment[] | undefined {
  if (!fromMeta || !existing || existing.length === 0) return fromMeta
  const known = new Map(
    existing
      .filter((a) => a.attachmentId)
      .map((a) => [a.attachmentId as string, a]),
  )
  return fromMeta.map((a) => known.get(a.attachmentId as string) ?? a)
}

/** Server ids of the chips already stored, in chip order. */
export function draftAttachmentIds(attachments: Attachment[]): string[] {
  return attachments.flatMap((a) => (a.attachmentId ? [a.attachmentId] : []))
}

/** `true` when the list has a chip the store does not hold yet. */
export function hasUnstoredDraftAttachments(
  attachments: Attachment[],
): boolean {
  return attachments.some((a) => a.file && !a.attachmentId)
}

/**
 * Stored chips a change dropped from the composer, when the user dropped them.
 *
 * A chip present before and gone after was removed by hand only for a
 * `remove`. On `submit`/`edit` the composer empties itself because the chips
 * went out on a message — deleting them would pull the bytes from under the
 * message that now references them (the store would refuse, but the attempt
 * is still wrong). `attach` and `hydrate` never drop anything.
 */
export function removedStoredAttachmentIds(
  previous: Attachment[],
  next: Attachment[],
  reason: DraftAttachmentChangeReason,
): string[] {
  if (reason !== 'remove') return []
  const kept = new Set(next.map((a) => a.id))
  return previous.flatMap((a) =>
    a.attachmentId && !kept.has(a.id) ? [a.attachmentId] : [],
  )
}

/**
 * Release the stored bytes of every chip the user removed from the composer.
 * Fire-and-forget from the caller's point of view: a failure leaves an orphan
 * in the store, which is a leak and not a broken draft, so it is logged in
 * DEV and nothing else. Resolves with the ids a delete was issued for.
 */
export async function releaseRemovedDraftAttachments(
  sessionId: string,
  previous: Attachment[],
  next: Attachment[],
  reason: DraftAttachmentChangeReason,
  remove: (input: {
    session_id: string
    attachment_id: string
  }) => Promise<unknown> = deleteAttachment,
): Promise<string[]> {
  const removed = removedStoredAttachmentIds(previous, next, reason)
  await Promise.all(
    removed.map((attachmentId) =>
      remove({ session_id: sessionId, attachment_id: attachmentId }).catch(
        (err) => {
          if (import.meta.env.DEV) {
            console.warn(
              `[attachments] delete-attachment failed for ${attachmentId}`,
              err,
            )
          }
        },
      ),
    ),
  )
  return removed
}

/**
 * Fold what is known about each chip into the composer's own list.
 *
 * The composer is the source of truth for WHICH chips there are; the hook
 * learns things about them later — the server id once the upload lands, the
 * bytes and thumbnail once a restored chip is hydrated. Those fields are
 * copied onto the matching chip only where the composer's copy lacks them;
 * chips are never added or removed here. Returns `current` itself when
 * nothing changed, so an effect can bail out without a re-render.
 */
export function mergeSyncedAttachments(
  current: Attachment[],
  synced: readonly Attachment[],
): Attachment[] {
  if (synced.length === 0 || current.length === 0) return current
  const byId = new Map(synced.map((a) => [a.id, a]))
  let changed = false
  const next = current.map((a) => {
    const known = byId.get(a.id)
    if (!known) return a
    const patch: Partial<Attachment> = {}
    if (known.attachmentId && !a.attachmentId) {
      patch.attachmentId = known.attachmentId
    }
    if (known.file && !a.file) patch.file = known.file
    if (known.dataUrl && !a.dataUrl) patch.dataUrl = known.dataUrl
    if (Object.keys(patch).length === 0) return a
    changed = true
    return { ...a, ...patch }
  })
  return changed ? next : current
}
