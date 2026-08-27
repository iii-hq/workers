import { type FSWatcher, watch as fsWatch } from 'node:fs'
import { basename, dirname } from 'node:path'

export interface ProjectLocation {
  file: string
  namespace: string
  stateDir: string
}

export type ChangeKind = 'state' | 'file'

export interface ChangedEvent {
  kind: ChangeKind
  file: string
  namespace: string
  state_dir: string
  path: string
  captured_at: number
}

type WatchFn = (path: string, listener: (event: string, filename: string | Buffer | null) => void) => FSWatcher

export interface WatcherOptions {
  locate: () => Promise<ProjectLocation | null>
  emit: (event: ChangedEvent) => void
  coalesceMs?: number
  watchFn?: WatchFn
  now?: () => number
  log?: (message: string) => void
}

export interface StateWatcher {
  ensure(): Promise<ProjectLocation | null>
  location(): ProjectLocation | null
  close(): void
}

export const STATE_FILE = 'state.json'

export function createStateWatcher(options: WatcherOptions): StateWatcher {
  const coalesceMs = options.coalesceMs ?? 200
  const watchFn: WatchFn = options.watchFn ?? ((path, listener) => fsWatch(path, listener))
  const now = options.now ?? Date.now
  const log = options.log ?? (() => {})

  let current: ProjectLocation | null = null
  let watchers: FSWatcher[] = []
  let pending: Promise<ProjectLocation | null> | null = null
  const timers = new Map<ChangeKind, { timer: NodeJS.Timeout; path: string }>()

  function schedule(kind: ChangeKind, path: string) {
    const location = current
    if (!location) return
    const existing = timers.get(kind)
    if (existing) clearTimeout(existing.timer)
    const timer = setTimeout(() => {
      timers.delete(kind)
      options.emit({
        kind,
        file: location.file,
        namespace: location.namespace,
        state_dir: location.stateDir,
        path,
        captured_at: now(),
      })
    }, coalesceMs)
    timers.set(kind, { timer, path })
  }

  function stopWatchers() {
    for (const watcher of watchers) watcher.close()
    watchers = []
    for (const entry of timers.values()) clearTimeout(entry.timer)
    timers.clear()
  }

  function drop(reason: string) {
    log(`watch dropped: ${reason}`)
    stopWatchers()
    current = null
  }

  function arm(location: ProjectLocation) {
    const stateWatcher = watchFn(location.stateDir, (_event, filename) => {
      const name = filename == null ? null : filename.toString()
      if (name !== null && name !== STATE_FILE) return
      schedule('state', name ?? STATE_FILE)
    })
    const composeName = basename(location.file)
    const fileWatcher = watchFn(dirname(location.file), (_event, filename) => {
      const name = filename == null ? null : filename.toString()
      if (name !== null && name !== composeName) return
      schedule('file', composeName)
    })
    for (const watcher of [stateWatcher, fileWatcher]) {
      watcher.on('error', (error) => drop(String(error)))
    }
    watchers = [stateWatcher, fileWatcher]
    current = location
  }

  async function ensure(): Promise<ProjectLocation | null> {
    if (current) return current
    if (pending) return pending
    pending = (async () => {
      try {
        const location = await options.locate()
        if (!location) return null
        try {
          arm(location)
        } catch (error) {
          drop(String(error))
          return null
        }
        return location
      } finally {
        pending = null
      }
    })()
    return pending
  }

  return {
    ensure,
    location: () => current,
    close: () => {
      stopWatchers()
      current = null
    },
  }
}
