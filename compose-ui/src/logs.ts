import { open } from 'node:fs/promises'
import { join } from 'node:path'

export const CONTAINER_NAME = /^[a-z0-9][a-z0-9_.-]*$/
export const DEFAULT_LINES = 200
export const MAX_LINES = 500
export const MAX_BYTES = 256 * 1024

export interface LogTail {
  container: string
  path: string
  lines: string[]
  size: number
  truncated: boolean
  missing: boolean
}

export function logPath(stateDir: string, container: string): string {
  if (!CONTAINER_NAME.test(container)) {
    throw new Error(`INVALID_CONTAINER: ${JSON.stringify(container)} is not a compose container name`)
  }
  return join(stateDir, 'logs', `${container}.log`)
}

export function clampLines(requested: unknown): number {
  const value = typeof requested === 'number' && Number.isFinite(requested) ? Math.trunc(requested) : DEFAULT_LINES
  return Math.min(MAX_LINES, Math.max(1, value))
}

function lastLines(text: string, count: number, droppedHead: boolean): { lines: string[]; total: number } {
  const lines = text.split('\n')
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop()
  if (droppedHead && lines.length > 0) lines.shift()
  const total = lines.length
  return { lines: total > count ? lines.slice(total - count) : lines, total }
}

export async function readLogTail(stateDir: string, container: string, count = DEFAULT_LINES): Promise<LogTail> {
  const path = logPath(stateDir, container)
  const wanted = clampLines(count)
  let handle: Awaited<ReturnType<typeof open>>
  try {
    handle = await open(path, 'r')
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return { container, path, lines: [], size: 0, truncated: false, missing: true }
    }
    throw error
  }
  try {
    const { size } = await handle.stat()
    const length = Math.min(size, MAX_BYTES)
    const buffer = Buffer.alloc(length)
    if (length > 0) await handle.read(buffer, 0, length, size - length)
    const clipped = size > length
    const { lines, total } = lastLines(buffer.toString('utf8'), wanted, clipped)
    const truncated = clipped || total > wanted
    return { container, path, lines, size, truncated, missing: false }
  } finally {
    await handle.close()
  }
}
