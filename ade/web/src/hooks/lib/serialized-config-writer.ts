/**
 * Any server-held JSON object that is replaced wholesale on write — the
 * `console` configuration entry, the workspace layout document, …
 */
export type SerializedValue = Record<string, unknown>

export type ConfigTransform = (value: SerializedValue) => SerializedValue

interface PendingConfigWrite {
  revision: number
  transform: ConfigTransform
}

export interface SerializedConfigWriterOptions {
  readRemote: () => Promise<SerializedValue | null>
  writeRemote: (value: SerializedValue) => Promise<void>
  readCached: () => SerializedValue | null | undefined
  publish: (value: SerializedValue) => void
  cancelReads?: () => void
}

/**
 * Serializes one document's read-modify-write cycle (the console
 * configuration entry, the workspace layout).
 *
 * Each queued transform reads again only after the previous write has
 * settled, so it rebases on the latest remote value. When an older write
 * completes, every newer optimistic transform is replayed before publishing
 * to the cache; an ACK can therefore never roll the UI back to its own older
 * snapshot.
 */
export class SerializedConfigWriter {
  private readonly pending: PendingConfigWrite[] = []
  private tail: Promise<void> = Promise.resolve()
  private optimistic: SerializedValue | undefined
  private issuedRevision = 0
  private settledRevision = 0

  constructor(private readonly options: SerializedConfigWriterOptions) {}

  enqueue(
    transform: ConfigTransform,
    fallback: SerializedValue = {},
  ): SerializedValue {
    const cached = this.options.readCached()
    const base =
      this.pending.length > 0
        ? (this.optimistic ?? cached ?? fallback)
        : (cached ?? fallback)
    const optimistic = transform(base)
    const entry = {
      revision: ++this.issuedRevision,
      transform,
    }

    this.optimistic = optimistic
    this.pending.push(entry)
    this.options.cancelReads?.()
    this.tail = this.tail.then(() => this.commit(entry))
    return optimistic
  }

  /** Query reads crossing a local write keep the newest optimistic value. */
  async readForQuery(): Promise<SerializedValue | null> {
    const issuedAtStart = this.issuedRevision
    const settledAtStart = this.settledRevision
    const remote = await this.options.readRemote()
    const crossedLocalWrite =
      issuedAtStart !== this.issuedRevision ||
      settledAtStart !== this.settledRevision

    if (crossedLocalWrite || this.pending.length > 0) {
      return this.optimistic ?? this.options.readCached() ?? remote
    }
    return remote
  }

  /** Makes tests (and future explicit flush callers) wait for the queue. */
  async whenIdle(): Promise<void> {
    await this.tail
  }

  private async commit(entry: PendingConfigWrite): Promise<void> {
    try {
      const current = (await this.options.readRemote()) ?? {}
      const committed = entry.transform(current)
      await this.options.writeRemote(committed)

      this.removePending(entry.revision)
      this.settledRevision = entry.revision
      const visible = this.pending.reduce(
        (value, pending) => pending.transform(value),
        committed,
      )
      this.optimistic = visible

      // Queries from any observer sharing this document's query key may have started
      // while the write was in flight. Cancel them before publishing the
      // rebased optimistic value.
      this.options.cancelReads?.()
      this.options.publish(visible)
    } catch {
      this.removePending(entry.revision)
      this.settledRevision = entry.revision
      this.options.cancelReads?.()
      // Preserve the existing best-effort behavior on failure. A later
      // queued write still fetches fresh remote state; otherwise the next
      // successful poll reconciles the optimistic cache.
    }
  }

  private removePending(revision: number): void {
    const index = this.pending.findIndex((entry) => entry.revision === revision)
    if (index >= 0) this.pending.splice(index, 1)
  }
}
