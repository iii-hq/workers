import { describe, expect, it } from 'vitest'
import type { ConsoleConfigValue } from '@/lib/console-config'
import { SerializedConfigWriter } from './serialized-config-writer'

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  const promise = new Promise<T>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

function numeric(value: ConsoleConfigValue, key: string): number {
  const candidate = value[key]
  return typeof candidate === 'number' ? candidate : 0
}

describe('SerializedConfigWriter', () => {
  it('serializes writes, rebases each transform, and never publishes an older optimistic state', async () => {
    let cache: ConsoleConfigValue = { count: 0, external: 'initial' }
    let remote: ConsoleConfigValue = { ...cache }
    let readCount = 0
    let writeCount = 0
    let activeWrites = 0
    let maxActiveWrites = 0
    const publishedCounts: number[] = []
    const firstWriteStarted = deferred<void>()
    const releaseFirstWrite = deferred<void>()

    const writer = new SerializedConfigWriter({
      readRemote: async () => {
        readCount += 1
        // Simulate another config writer landing between our two commits.
        return readCount === 2
          ? { ...remote, external: 'fresh' }
          : { ...remote }
      },
      writeRemote: async (value) => {
        writeCount += 1
        activeWrites += 1
        maxActiveWrites = Math.max(maxActiveWrites, activeWrites)
        if (writeCount === 1) {
          firstWriteStarted.resolve()
          await releaseFirstWrite.promise
        }
        remote = { ...value }
        activeWrites -= 1
      },
      readCached: () => cache,
      publish: (value) => {
        cache = value
        publishedCounts.push(numeric(value, 'count'))
      },
    })

    const increment = (value: ConsoleConfigValue) => ({
      ...value,
      count: numeric(value, 'count') + 1,
    })
    cache = writer.enqueue(increment)
    cache = writer.enqueue(increment)

    await firstWriteStarted.promise
    expect(numeric(cache, 'count')).toBe(2)
    expect(writeCount).toBe(1)

    releaseFirstWrite.resolve()
    await writer.whenIdle()

    expect(maxActiveWrites).toBe(1)
    expect(readCount).toBe(2)
    expect(writeCount).toBe(2)
    expect(remote).toMatchObject({ count: 2, external: 'fresh' })
    // The first ACK is count=1, but the second pending transform is replayed
    // before it reaches the cache, so the visible state never rolls back.
    expect(publishedCounts).toEqual([2, 2])
  })

  it('ignores a query response that started before a newer optimistic write', async () => {
    let cache: ConsoleConfigValue = { count: 0 }
    let remote: ConsoleConfigValue = { count: 0 }
    let readCount = 0
    const staleQuery = deferred<ConsoleConfigValue | null>()

    const writer = new SerializedConfigWriter({
      readRemote: async () => {
        readCount += 1
        if (readCount === 1) return staleQuery.promise
        return { ...remote }
      },
      writeRemote: async (value) => {
        remote = { ...value }
      },
      readCached: () => cache,
      publish: (value) => {
        cache = value
      },
    })

    const queryResult = writer.readForQuery()
    cache = writer.enqueue((value) => ({
      ...value,
      count: numeric(value, 'count') + 1,
    }))
    await writer.whenIdle()

    staleQuery.resolve({ count: 0 })

    await expect(queryResult).resolves.toMatchObject({ count: 1 })
    expect(cache).toMatchObject({ count: 1 })
    expect(remote).toMatchObject({ count: 1 })
  })
})
