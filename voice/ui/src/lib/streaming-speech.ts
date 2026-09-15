import type { SessionMessageEntry } from './types'

export interface PreparedSpeech { text: string; max_chunk_chars: number }
export interface SpeechSnapshot extends SessionMessageEntry {
  session_id: string
  timestamp: number
  revision?: number
}
interface Entry {
  raw: string
  revision: number
  spoken: string
  complete: boolean
}

/** Keep whole words and complete sentences; long sentences use bounded phrases.
 * The trailing word is held until a later snapshot or the final flush. */
export function speechChunks(text: string, complete: boolean, limit: number): { chunks: string[]; consumed: string } {
  if (!Number.isFinite(limit) || limit < 1) throw new Error('Invalid streaming speech character limit.')
  const cap = Math.max(1, Math.min(240, Math.floor(limit)))
  const chars = [...text]
  const chunks: string[] = []
  let offset = 0
  while (offset < chars.length) {
    let end = 0
    const available = chars.length - offset
    for (let i = 1; i <= Math.min(cap, available); i++) {
      const previous = chars[offset + i - 1]
      const next = chars[offset + i]
      if (/[.!?。！？]/u.test(previous) && (next === ' ' || (complete && next === undefined))) { end = i; break }
    }
    if (!end && (available > cap || complete)) {
      const bound = Math.min(cap, available)
      if (complete && available <= cap) end = available
      else {
        for (let i = bound; i > 0; i--) if (/\s/u.test(chars[offset + i] ?? '')) { end = i; break }
        // A single unbroken token must still respect the backend's character cap.
        if (!end && available > cap) end = cap
      }
    }
    if (!end) break
    const chunk = chars.slice(offset, offset + end).join('').trim()
    offset += end
    if (chunk) chunks.push(chunk)
  }
  return { chunks, consumed: chars.slice(0, offset).join('') }
}

/** Latest full snapshots, never token deltas. One preparation at a time, with
 * coalescing under rapid token delivery. Audio is queued by the caller. */
export class StreamingSpeech {
  private entries = new Map<string, Entry>()
  private dirty = new Set<string>()
  private current: string | null = null
  private timestamp = -Infinity
  private timer: ReturnType<typeof setTimeout> | undefined
  private running = false
  private active = true
  constructor(private options: {
    prepare: (text: string, complete: boolean) => Promise<PreparedSpeech>
    onChunk: (text: string) => void
    onError: (error: unknown) => void
  }) {}

  update(event: SpeechSnapshot, complete = false) {
    if (!this.active || event.message?.role !== 'assistant' || event.elided) return
    const raw = (event.message.content ?? []).filter((b) => b.type === 'text').map((b) => b.text ?? '').join('\n')
    const previous = this.entries.get(event.entry_id)
    const revision = event.revision ?? 0
    if (!Number.isSafeInteger(revision) || revision < 0 || (previous && revision <= previous.revision && !complete)) return
    if (!previous) {
      if (event.timestamp < this.timestamp) return
      if (this.entries.size >= 128 || raw.length > 262144) { this.fail(new Error('Streaming read-aloud message limit reached.')); return }
      if (this.current) this.finishEntry(this.current)
      this.current = event.entry_id
      this.entries.set(event.entry_id, { raw, revision, spoken: '', complete })
    } else {
      previous.raw = raw; previous.revision = revision; previous.complete ||= complete
    }
    this.timestamp = Math.max(this.timestamp, event.timestamp)
    this.dirty.add(event.entry_id)
    this.schedule()
  }

  private finishEntry(id: string) {
    const entry = this.entries.get(id)
    if (entry) { entry.complete = true; this.dirty.add(id) }
  }

  finish(reply?: { id: string; text: string } | null) {
    if (!this.active) return
    if (reply) {
      const entry = this.entries.get(reply.id)
      if (entry) { entry.raw = reply.text; this.finishEntry(reply.id) }
      else this.update({ session_id: '', entry_id: reply.id, timestamp: this.timestamp,
        message: { role: 'assistant', content: [{ type: 'text', text: reply.text }] } }, true)
    }
    for (const id of this.entries.keys()) this.finishEntry(id)
    this.schedule()
  }

  private schedule() {
    if (!this.active || this.running || this.timer !== undefined) return
    this.timer = setTimeout(() => { this.timer = undefined; void this.drain() }, 120)
  }

  private async drain() {
    this.running = true
    try {
      while (this.active && this.dirty.size) {
        const id = this.dirty.values().next().value!
        this.dirty.delete(id)
        const entry = this.entries.get(id)!
        const raw = entry.raw, complete = entry.complete
        if (raw.length > 262144) throw new Error('Streaming read-aloud message limit reached.')
        const prepared = await this.options.prepare(raw, complete)
        if (!this.active) return
        if (!entry.raw.startsWith(raw)) { this.dirty.add(id); continue }
        // Already-queued audio cannot be unsaid. Never replay or splice an edit
        // into its old prefix; stop with a visible error instead.
        if (!prepared.text.startsWith(entry.spoken)) throw new Error('The streamed reply changed after it was read. Read the final reply manually.')
        const { chunks, consumed } = speechChunks(prepared.text.slice(entry.spoken.length), complete, prepared.max_chunk_chars)
        entry.spoken += consumed
        for (const chunk of chunks) {
          if (!this.active) return
          this.options.onChunk(chunk)
        }
      }
    } catch (error) { this.fail(error) }
    finally { this.running = false; if (this.dirty.size) this.schedule() }
  }

  private fail(error: unknown) { this.stop(); this.options.onError(error) }
  stop() {
    this.active = false
    if (this.timer !== undefined) clearTimeout(this.timer)
    this.entries.clear(); this.dirty.clear()
  }
}
