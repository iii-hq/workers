// Run: node --test src/lib/frameFollower.test.ts (Node >= 23.6 strips types).
import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  type AppliedFrame,
  createFrameFollower,
  type FrameChangeNote,
  parseFrameChange,
  type StoredFrame,
} from './frameFollower.ts'

/** A fake worker: one stored frame per session, like computer::frame. */
function fakeWorker(epoch = 100) {
  const state = { epoch, seq: 0, delayMs: 0, reads: [] as Array<number | undefined> }
  let inFlight = 0
  let maxInFlight = 0
  return {
    state,
    maxInFlight: () => maxInFlight,
    store() {
      state.seq++
    },
    note(change: 'updated' | 'cleared' = 'updated', session = 'c1'): FrameChangeNote {
      return { session_id: session, epoch: state.epoch, frame_seq: state.seq, change }
    },
    async read(since: number | undefined): Promise<StoredFrame | null> {
      state.reads.push(since)
      inFlight++
      maxInFlight = Math.max(maxInFlight, inFlight)
      try {
        if (state.delayMs) await new Promise((r) => setTimeout(r, state.delayMs))
        const unchanged = since !== undefined && since === state.seq
        return {
          frame: state.seq === 0 || unchanged ? undefined : `img-${state.epoch}-${state.seq}`,
          mime: 'image/jpeg',
          width: 1280,
          height: 800,
          frame_seq: state.seq,
          epoch: state.epoch,
        }
      } finally {
        inFlight--
      }
    },
  }
}

async function settle(f: { idle(): boolean }) {
  for (let i = 0; i < 400 && !f.idle(); i++) await new Promise((r) => setTimeout(r, 2))
  assert.ok(f.idle(), 'follower did not settle')
}

function follow(worker: ReturnType<typeof fakeWorker>, sessionId = 'c1') {
  const painted: AppliedFrame[] = []
  const follower = createFrameFollower({
    sessionId,
    read: (since) => worker.read(since),
    apply: (f) => painted.push(f),
  })
  return { follower, painted }
}

describe('frame follower', () => {
  it('paints the initial frame, then each update', async () => {
    const w = fakeWorker()
    w.store() // a frame exists before the viewer opens
    const { follower, painted } = follow(w)
    await follower.resync()
    assert.deepEqual(
      painted.map((p) => p.data),
      ['img-100-1'],
    )
    w.store()
    follower.notify(w.note())
    await settle(follower)
    w.store()
    follower.notify(w.note())
    await settle(follower)
    assert.deepEqual(
      painted.map((p) => p.frameSeq),
      [1, 2, 3],
    )
    // Follow-up reads pass the cursor so an unchanged frame costs no image.
    assert.deepEqual(w.state.reads, [undefined, 1, 2])
  })

  it('coalesces a burst during a slow read: one read in flight, newest wins', async () => {
    const w = fakeWorker()
    w.state.delayMs = 30
    const { follower, painted } = follow(w)
    w.store()
    follower.notify(w.note())
    for (let i = 0; i < 1000; i++) {
      w.store()
      follower.notify(w.note())
    }
    await settle(follower)
    assert.equal(w.maxInFlight(), 1)
    assert.ok(follower.stats.reads <= 2, `reads=${follower.stats.reads}`)
    assert.equal(painted.at(-1)?.frameSeq, 1001)
    assert.ok(painted.length <= 2)
  })

  it('ignores other sessions, duplicates, stale revisions and cleared', async () => {
    const w = fakeWorker()
    w.store()
    w.store()
    const { follower, painted } = follow(w)
    await follower.resync()
    assert.equal(painted.length, 1)
    follower.notify({ session_id: 'c2', epoch: 100, frame_seq: 9, change: 'updated' })
    follower.notify({ session_id: 'c1', epoch: 100, frame_seq: 2, change: 'updated' }) // duplicate
    follower.notify({ session_id: 'c1', epoch: 100, frame_seq: 1, change: 'updated' }) // stale
    follower.notify({ session_id: 'c1', epoch: 100, frame_seq: 2, change: 'cleared' })
    await settle(follower)
    assert.equal(follower.stats.skipped, 4)
    assert.equal(w.state.reads.length, 1)
    assert.equal(painted.length, 1)
  })

  it('a new epoch (worker restart) supersedes, with a full read', async () => {
    const w = fakeWorker(100)
    for (let i = 0; i < 5; i++) w.store()
    const { follower, painted } = follow(w)
    await follower.resync()
    assert.equal(painted.at(-1)?.frameSeq, 5)
    // Restarted worker: seq restarts at 1 under a newer epoch.
    w.state.epoch = 200
    w.state.seq = 0
    w.store()
    follower.notify(w.note())
    await settle(follower)
    assert.deepEqual(
      [painted.at(-1)?.epoch, painted.at(-1)?.frameSeq],
      [200, 1],
    )
    assert.equal(w.state.reads.at(-1), undefined)
  })

  it('a failed read is not retried on a timer and recovers on the next notification', async () => {
    const w = fakeWorker()
    w.store()
    let fail = true
    const errors: unknown[] = []
    const painted: AppliedFrame[] = []
    const follower = createFrameFollower({
      sessionId: 'c1',
      read: (since) => (fail ? Promise.reject(new Error('offline')) : w.read(since)),
      apply: (f) => painted.push(f),
      onError: (e) => errors.push(e),
    })
    await follower.resync()
    assert.equal(errors.length, 1)
    assert.equal(painted.length, 0)
    fail = false
    w.store()
    follower.notify(w.note())
    await settle(follower)
    assert.equal(painted.at(-1)?.frameSeq, 2)
  })

  it('stops cleanly: nothing is read or painted after stop', async () => {
    const w = fakeWorker()
    const { follower, painted } = follow(w)
    follower.stop()
    w.store()
    follower.notify(w.note())
    await follower.resync()
    assert.equal(w.state.reads.length, 0)
    assert.equal(painted.length, 0)
  })

  it('parses only well-formed notifications', () => {
    assert.deepEqual(
      parseFrameChange({
        session_id: 'c1',
        epoch: 1,
        frame_seq: 2,
        change: 'updated',
        width: 1,
        height: 1,
      }),
      { session_id: 'c1', epoch: 1, frame_seq: 2, change: 'updated' },
    )
    assert.equal(parseFrameChange({ event: { data: {} } }), null)
    assert.equal(parseFrameChange({ session_id: 'c1', epoch: 1, frame_seq: 2, change: 'x' }), null)
    assert.equal(parseFrameChange(null), null)
  })
})
