/**
 * Which server-queued rows belong in the person's queued-messages strip.
 *
 * `harness::status` → `queued` parks everything that arrived mid-stream: the
 * person's own sends (this tab or another), and the harness talking to the
 * agent — observer and system notifications, a sub-agent's opening task.
 * Those are not the person's messages: showing them as "Queued" above the
 * composer, with the ↑-to-edit hint beside them, reads as a message they
 * never wrote. Only `direct` rows (and rows from a harness that predates the
 * channel field) are theirs.
 */

import type { QueuedMessagePreview } from '../../lib/backend/types'
import { isNotificationEntryId } from '../../lib/sessions/entry-mapper'

/** A row is the person's when the harness parked it as `direct` (or predates
    the channel field), and it carries no notification mark: an
    `origin.notification` or a notification entry id (`e_notify_`,
    `e_fire_`, …), which older harnesses send without a channel. */
export function isPersonQueuedRow(
  row: Pick<QueuedMessagePreview, 'channel'> &
    Partial<Pick<QueuedMessagePreview, 'id' | 'origin'>>,
): boolean {
  if (row.channel !== undefined && row.channel !== 'direct') return false
  if ((row.origin as { notification?: unknown } | undefined)?.notification === true)
    return false
  return !(row.id !== undefined && isNotificationEntryId(row.id))
}
