/**
 * Stop with messages still queued.
 *
 * A message sent while a reply streams parks in the harness queue and
 * drains into the transcript when the turn's step ends. A Stop ends the turn
 * as cancelled, and the harness deliberately starts no new turn after it
 * (`finalize_cancelled`: "the user stopped this turn"), so a queued message
 * used to drain as a sent-looking bubble that nothing would ever answer.
 *
 * Stop means stop, so nothing the person queued is sent on its own: before
 * the stop reaches the harness, each of their still-parked messages is
 * taken back out of the queue (`harness::unqueue`) and returned to the
 * composer as a draft, with a notice saying so. A row the harness already
 * took (it drained into the running turn first) stays where it went.
 */

/** One of the person's queued messages, by its transcript entry id. */
export interface QueuedRow {
  id: string
  text: string
}

/**
 * Take `rows` back out of the queue, all at once, and sort them by outcome:
 * `restored` were still parked and are the composer's again; `kept` had
 * already gone to the agent (`false`) or could not be taken back (the call
 * failed), so they will still be delivered. An answer without the flag
 * counts as taken back.
 */
export async function pullBackQueued<Row extends QueuedRow>(
  rows: readonly Row[],
  unqueue: (entryId: string) => Promise<boolean | undefined>,
): Promise<{ restored: Row[]; kept: Row[] }> {
  const outcomes = await Promise.all(
    rows.map((row) =>
      unqueue(row.id).then(
        (removed) => removed !== false,
        () => false,
      ),
    ),
  )
  return {
    restored: rows.filter((_, index) => outcomes[index]),
    kept: rows.filter((_, index) => !outcomes[index]),
  }
}

/**
 * The composer text once the pulled-back messages return: each one in the
 * order it was queued, then whatever was being typed, a blank line apart.
 * A message with only attachments adds no text.
 */
export function draftWithPulledBack(
  typed: string | undefined,
  pulled: readonly QueuedRow[],
): string {
  const parts = pulled
    .map((row) => row.text.trim())
    .filter((text) => text.length > 0)
  const current = typed?.trim()
  if (current) parts.push(current)
  return parts.join('\n\n')
}

/** The notice that says where the pulled-back messages went. */
export function pulledBackNotice(count: number): string {
  return count === 1
    ? 'Your queued message was not sent. It is back in the composer.'
    : `Your ${count} queued messages were not sent. They are back in the composer.`
}
