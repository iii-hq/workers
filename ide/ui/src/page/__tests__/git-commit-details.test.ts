import type { Host } from '@iii-dev/console-ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { DiffContents } from '../diff-load'
import type { DiffSource } from '../diff-source'
import { useCommitFileDiff } from '../GitDiffPreview'
import { filesView } from '../GitLogTab'
import { mount } from './bare-hooks'

vi.mock('react', async (original) => ({
  ...(await original<typeof import('react')>()),
  ...(await import('./bare-hooks')).hooks,
}))
// The real components are supplied by the Console import map, not Node.
vi.mock('@iii-dev/console-ui', () => ({}))

// Each read the preview asked for, settled by the test.
const reads = vi.hoisted(
  () =>
    [] as Array<{
      path: string
      source: DiffSource
      resolve(contents: DiffContents): void
      reject(error: Error): void
    }>,
)
vi.mock('../diff-load', () => ({
  loadDiffContents: (_host: Host, _root: string, path: string, source: DiffSource) =>
    new Promise<DiffContents>((resolve, reject) => reads.push({ path, source, resolve, reject })),
}))

const host = {} as Host
const body = (text: string): DiffContents => ({ oldContents: '', newContents: text })

describe('the Log files view', () => {
  it('keeps the diff preview off unless it was turned on', () => {
    expect(filesView(undefined)).toEqual({ height: null, grouped: true, info: true, preview: false })
    expect(filesView({ grouped: false, info: false, preview: 'yes' })).toMatchObject({ preview: false })
    expect(filesView({ preview: true, height: 47.6 })).toEqual({
      height: 48,
      grouped: true,
      info: true,
      preview: true,
    })
  })
})

describe('the diff preview', () => {
  // The read waits for the pick to rest: settling lets that wait pass and
  // the reads that came back land.
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })
  const settle = () => vi.advanceTimersByTimeAsync(150)

  it("reads the commit's change to the file, then swaps when another is picked", async () => {
    reads.length = 0
    const view = mount(
      (props: { path: string }) => useCommitFileDiff(host, '/repo', props.path, 'c0ffee', 'beef', undefined),
      { path: 'a.ts' },
    )
    expect(view.result.state).toEqual({ phase: 'loading' })
    await settle()
    expect(reads[0]).toMatchObject({
      path: 'a.ts',
      source: { type: 'commit', sha: 'c0ffee', parent: 'beef', from: undefined },
    })

    reads[0].resolve(body('a'))
    await settle()
    expect(view.result.state).toEqual({ phase: 'ready', contents: body('a') })

    // Another file shows loading, not the last one's diff.
    view.rerender({ path: 'b.ts' })
    expect(view.result.state).toEqual({ phase: 'loading' })
    await settle()
    // A read that lands after a third file was picked is dropped.
    view.rerender({ path: 'c.ts' })
    reads[1].resolve(body('b'))
    await settle()
    expect(view.result.state).toEqual({ phase: 'loading' })
    reads[2].resolve(body('c'))
    await settle()
    expect(view.result.state).toEqual({ phase: 'ready', contents: body('c') })
    expect(reads.map((read) => read.path)).toEqual(['a.ts', 'b.ts', 'c.ts'])
    view.unmount()
  })

  it('reads only the file the pick rests on', async () => {
    reads.length = 0
    const view = mount(
      (props: { path: string }) => useCommitFileDiff(host, '/repo', props.path, 'c0ffee', 'beef', undefined),
      { path: 'a.ts' },
    )
    // Arrowing down the list: a new file every 100 ms, none read.
    await vi.advanceTimersByTimeAsync(100)
    view.rerender({ path: 'b.ts' })
    await vi.advanceTimersByTimeAsync(100)
    view.rerender({ path: 'c.ts' })
    await vi.advanceTimersByTimeAsync(100)
    expect(reads).toHaveLength(0)
    await settle()
    expect(reads.map((read) => read.path)).toEqual(['c.ts'])
    view.unmount()
  })

  it('says why a read failed, and reads again on reload', async () => {
    reads.length = 0
    const view = mount(() => useCommitFileDiff(host, '/repo', 'a.ts', 'c0ffee', null, 'old.ts'), {})
    await settle()
    expect(reads[0].source).toEqual({ type: 'commit', sha: 'c0ffee', parent: null, from: 'old.ts' })
    reads[0].reject(new Error('unknown revision: c0ffee'))
    await settle()
    expect(view.result.state).toEqual({ phase: 'error', message: 'unknown revision: c0ffee' })

    view.result.reload()
    expect(view.result.state).toEqual({ phase: 'loading' })
    await settle()
    expect(reads).toHaveLength(2)
    reads[1].resolve(body('a'))
    await settle()
    expect(view.result.state).toEqual({ phase: 'ready', contents: body('a') })
    view.unmount()
  })
})
