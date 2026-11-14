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

export function isPersonQueuedRow(
  row: Pick<QueuedMessagePreview, 'channel'>,
): boolean {
  return row.channel === undefined || row.channel === 'direct'
}
