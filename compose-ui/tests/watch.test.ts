import { EventEmitter } from 'node:events'
import type { FSWatcher } from 'node:fs'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { type ChangedEvent, createStateWatcher, STATE_FILE } from '../src/watch.js'

class FakeWatcher extends EventEmitter {
  closed = false
  constructor(
    public path: string,
    public listener: (event: string, filename: string | Buffer | null) => void,
  ) {
    super()
  }
  close() {
    this.closed = true
  }
}

const location = { file: '/proj/worker-compose.yaml', namespace: 'app', stateDir: '/home/me/.iii/compose/app/proj-1' }

function harness(locate = vi.fn(async () => location)) {
  const watchers: FakeWatcher[] = []
  const events: ChangedEvent[] = []
  const watcher = createStateWatcher({
    locate,
    emit: (event) => events.push(event),
    coalesceMs: 50,
    now: () => 1_700_000_000_000,
    watchFn: (path, listener) => {
      const fake = new FakeWatcher(path, listener)
      watchers.push(fake)
      return fake as unknown as FSWatcher
    },
  })
  return { watcher, watchers, events, locate }
}

describe('createStateWatcher', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('arms one watcher on the state directory and one on the compose file directory', async () => {
    const { watcher, watchers } = harness()
    await expect(watcher.ensure()).resolves.toEqual(location)
    expect(watchers.map((w) => w.path)).toEqual([location.stateDir, '/proj'])
  })

  it('locates once for concurrent ensure calls', async () => {
    const { watcher, locate } = harness()
    await Promise.all([watcher.ensure(), watcher.ensure(), watcher.ensure()])
    expect(locate).toHaveBeenCalledTimes(1)
    await watcher.ensure()
    expect(locate).toHaveBeenCalledTimes(1)
  })

  it('answers null without arming when the daemon is unreachable', async () => {
    const { watcher, watchers } = harness(vi.fn(async () => null))
    await expect(watcher.ensure()).resolves.toBeNull()
    expect(watchers).toHaveLength(0)
    expect(watcher.location()).toBeNull()
  })

  it('coalesces a burst of state.json writes into one state event', async () => {
    const { watcher, watchers, events } = harness()
    await watcher.ensure()
    const [state] = watchers
    state.listener('change', STATE_FILE)
    state.listener('change', STATE_FILE)
    state.listener('rename', STATE_FILE)
    vi.advanceTimersByTime(49)
    expect(events).toHaveLength(0)
    vi.advanceTimersByTime(1)
    expect(events).toEqual([
      {
        kind: 'state',
        file: location.file,
        namespace: location.namespace,
        state_dir: location.stateDir,
        path: STATE_FILE,
        captured_at: 1_700_000_000_000,
      },
    ])
  })

  it('ignores other files in the state directory', async () => {
    const { watcher, watchers, events } = harness()
    await watcher.ensure()
    watchers[0].listener('change', 'engine.log')
    vi.advanceTimersByTime(100)
    expect(events).toHaveLength(0)
  })

  it('reports compose file edits as file events and ignores siblings', async () => {
    const { watcher, watchers, events } = harness()
    await watcher.ensure()
    const [, fileWatcher] = watchers
    fileWatcher.listener('change', 'README.md')
    fileWatcher.listener('change', 'worker-compose.yaml')
    vi.advanceTimersByTime(50)
    expect(events.map((e) => [e.kind, e.path])).toEqual([['file', 'worker-compose.yaml']])
  })

  it('drops the watch on a watcher error and relocates on the next ensure', async () => {
    const { watcher, watchers, locate } = harness()
    await watcher.ensure()
    watchers[0].emit('error', new Error('EBADF'))
    expect(watcher.location()).toBeNull()
    expect(watchers.every((w) => w.closed)).toBe(true)
    await watcher.ensure()
    expect(locate).toHaveBeenCalledTimes(2)
    expect(watchers).toHaveLength(4)
  })

  it('close stops every watcher and pending timer', async () => {
    const { watcher, watchers, events } = harness()
    await watcher.ensure()
    watchers[0].listener('change', STATE_FILE)
    watcher.close()
    vi.advanceTimersByTime(100)
    expect(events).toHaveLength(0)
    expect(watchers.every((w) => w.closed)).toBe(true)
  })
})
