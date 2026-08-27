import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { beforeAll, describe, expect, it } from 'vitest'
import { clampLines, DEFAULT_LINES, logPath, MAX_BYTES, MAX_LINES, readLogTail } from '../src/logs.js'

let stateDir: string

beforeAll(async () => {
  stateDir = await mkdtemp(join(tmpdir(), 'compose-ui-logs-'))
  await mkdir(join(stateDir, 'logs'))
  await writeFile(join(stateDir, 'logs', 'console.log'), ['one', 'two', 'three', 'four', ''].join('\n'))
  await writeFile(join(stateDir, 'logs', 'big.log'), `${'x'.repeat(100)}\n`.repeat(4000))
  await writeFile(join(stateDir, 'logs', 'nonewline.log'), 'alpha\nbeta')
})

describe('logPath', () => {
  it('joins the daemon log directory for a compose container name', () => {
    expect(logPath('/s', 'provider-openai')).toBe(join('/s', 'logs', 'provider-openai.log'))
  })

  it('rejects anything that is not a compose container name', () => {
    for (const bad of ['../engine', 'Console', 'a/b', '', '.hidden', 'a b']) {
      expect(() => logPath('/s', bad)).toThrow(/INVALID_CONTAINER/)
    }
  })
})

describe('clampLines', () => {
  it('defaults and clamps into the allowed range', () => {
    expect(clampLines(undefined)).toBe(DEFAULT_LINES)
    expect(clampLines(0)).toBe(1)
    expect(clampLines(-5)).toBe(1)
    expect(clampLines(12.9)).toBe(12)
    expect(clampLines(MAX_LINES + 1)).toBe(MAX_LINES)
    expect(clampLines(Number.NaN)).toBe(DEFAULT_LINES)
  })
})

describe('readLogTail', () => {
  it('returns the last lines with the trailing newline dropped', async () => {
    const tail = await readLogTail(stateDir, 'console', 2)
    expect(tail.lines).toEqual(['three', 'four'])
    expect(tail.truncated).toBe(true)
    expect(tail.missing).toBe(false)
    expect(tail.size).toBeGreaterThan(0)
  })

  it('returns every line and truncated=false when the file is short', async () => {
    const tail = await readLogTail(stateDir, 'console', 50)
    expect(tail.lines).toEqual(['one', 'two', 'three', 'four'])
    expect(tail.truncated).toBe(false)
  })

  it('keeps a final line without a newline', async () => {
    const tail = await readLogTail(stateDir, 'nonewline', 5)
    expect(tail.lines).toEqual(['alpha', 'beta'])
  })

  it('reads at most MAX_BYTES from the end and drops the partial first line', async () => {
    const tail = await readLogTail(stateDir, 'big', MAX_LINES)
    expect(tail.size).toBeGreaterThan(MAX_BYTES)
    expect(tail.truncated).toBe(true)
    expect(tail.lines).toHaveLength(MAX_LINES)
    expect(tail.lines.every((line) => line.length === 100)).toBe(true)
  })

  it('answers missing=true for a container that has not logged yet', async () => {
    const tail = await readLogTail(stateDir, 'quiet', 10)
    expect(tail).toMatchObject({ container: 'quiet', lines: [], size: 0, truncated: false, missing: true })
  })
})
