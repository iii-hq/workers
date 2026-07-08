import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  __resetFileIndexCacheForTests,
  fetchFileIndex,
  flattenTree,
  fuzzyFilterFiles,
  TREE_FUNCTION_ID,
  type TreeNodeWire,
} from './file-index'

const tree: TreeNodeWire = {
  name: 'repo',
  kind: 'dir',
  children: [
    { name: 'README.md', kind: 'file' },
    {
      name: 'src',
      kind: 'dir',
      children: [
        { name: 'main.rs', kind: 'file' },
        { name: 'weird (copy).rs', kind: 'file' },
        { name: 'link', kind: 'symlink' },
        {
          name: 'nested',
          kind: 'dir',
          children: [{ name: 'deep.ts', kind: 'file' }],
        },
      ],
    },
    { name: 'empty', kind: 'dir', children: [] },
    { name: 'no-children-dir', kind: 'dir' },
  ],
}

const flatTree = [
  'empty/',
  'no-children-dir/',
  'README.md',
  'src/',
  'src/main.rs',
  'src/nested/',
  'src/nested/deep.ts',
]

describe('flattenTree', () => {
  it('collects files and trailing-slash folders, skipping symlinks and paren paths', () => {
    expect(flattenTree(tree)).toEqual(flatTree)
  })

  it('returns [] for a childless root', () => {
    expect(flattenTree({ name: 'x', kind: 'dir' })).toEqual([])
  })
})

describe('fuzzyFilterFiles', () => {
  const index = [
    'src/lib/config.ts',
    'src/components/ConfigPanel.tsx',
    'harness/config.rs',
    'docs/notes.md',
  ]

  it('returns head of index for an empty query', () => {
    expect(fuzzyFilterFiles(index, '', 2)).toEqual(index.slice(0, 2))
  })

  it('ranks basename-prefix matches before substring matches', () => {
    const out = fuzzyFilterFiles(index, 'config')
    expect(out.slice(0, 2)).toEqual(['harness/config.rs', 'src/lib/config.ts'])
    expect(out).toContain('src/components/ConfigPanel.tsx')
  })

  it('matches subsequences as a last resort', () => {
    expect(fuzzyFilterFiles(index, 'dnm')).toEqual(['docs/notes.md'])
  })

  it('ranks folder basenames despite the trailing slash', () => {
    expect(
      fuzzyFilterFiles(['src/nested/deep.ts', 'src/nested/'], 'nes'),
    ).toEqual(['src/nested/', 'src/nested/deep.ts'])
  })

  it('excludes non-matches', () => {
    expect(fuzzyFilterFiles(index, 'zzz')).toEqual([])
  })
})

describe('fetchFileIndex', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    __resetFileIndexCacheForTests()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  const treeResponse = { path: '/w', root: tree }

  it('fetches through coder::tree with the working dir as base_dir', async () => {
    const trigger = vi.fn().mockResolvedValue(treeResponse)
    const index = await fetchFileIndex('/w', trigger)
    expect(index).toEqual(flatTree)
    expect(trigger).toHaveBeenCalledWith(
      TREE_FUNCTION_ID,
      expect.objectContaining({
        fs_scope: { root: '/w' },
        use_default_excludes: true,
      }),
    )
  })

  it('serves from cache within the TTL and refetches after it', async () => {
    const trigger = vi.fn().mockResolvedValue(treeResponse)
    await fetchFileIndex('/w', trigger)
    await fetchFileIndex('/w', trigger)
    expect(trigger).toHaveBeenCalledTimes(1)

    vi.advanceTimersByTime(31_000)
    await fetchFileIndex('/w', trigger)
    expect(trigger).toHaveBeenCalledTimes(2)
  })

  it('returns the last good index when a refetch fails', async () => {
    const trigger = vi
      .fn()
      .mockResolvedValueOnce(treeResponse)
      .mockRejectedValueOnce(new Error('worker away'))
    const first = await fetchFileIndex('/w', trigger)
    vi.advanceTimersByTime(31_000)
    const second = await fetchFileIndex('/w', trigger)
    expect(second).toEqual(first)
  })

  it('propagates the error when there is no cached index', async () => {
    const trigger = vi.fn().mockRejectedValue(new Error('worker away'))
    await expect(fetchFileIndex('/w', trigger)).rejects.toThrow('worker away')
  })
})
