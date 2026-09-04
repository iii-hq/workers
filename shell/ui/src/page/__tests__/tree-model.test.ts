import { describe, expect, it } from 'vitest'
import { flattenTree, type TreeNode } from '../coder'
import {
  applyTreeChanges,
  changedDirsOf,
  emptyTree,
  isDirLoaded,
  mergeSubtree,
  visibleTruncations,
} from '../tree-model'

function node(name: string, kind: TreeNode['kind'], children?: TreeNode[], truncated?: TreeNode['truncated']): TreeNode {
  return { name, kind, size: 0, mtime: 0, children, truncated }
}

const snapshot = node('root', 'dir', [
  node('src', 'dir', [
    node('a.ts', 'file'),
    node('deep', 'dir', undefined, { reason: 'max_depth', shown: 0, hint: '' }),
  ]),
  node('README.md', 'file'),
  node('node_modules', 'dir', undefined, { reason: 'default_exclude', shown: 0, hint: '' }),
])

describe('flattenTree loaded set', () => {
  it('marks listed folders loaded and cut-off folders not', () => {
    const tree = flattenTree(snapshot)
    expect(tree.paths).toEqual(['src/', 'src/a.ts', 'src/deep/', 'README.md', 'node_modules/'])
    expect(isDirLoaded(tree, 'src')).toBe(true)
    expect(isDirLoaded(tree, 'src/deep')).toBe(false)
    expect(isDirLoaded(tree, 'node_modules')).toBe(false)
    expect(isDirLoaded(tree, '')).toBe(true)
  })
})

describe('mergeSubtree', () => {
  it('splices a lazily fetched folder under its parent and marks it loaded', () => {
    const tree = flattenTree(snapshot)
    const sub = flattenTree(node('deep', 'dir', [node('x.ts', 'file'), node('y', 'dir', [])]))
    const next = mergeSubtree(tree, 'src/deep', sub)
    expect(next.paths).toContain('src/deep/x.ts')
    expect(next.paths).toContain('src/deep/y/')
    expect(next.kinds.get('src/deep/x.ts')).toBe('file')
    expect(isDirLoaded(next, 'src/deep')).toBe(true)
    expect(isDirLoaded(next, 'src/deep/y')).toBe(true)
    // Unrelated entries are untouched.
    expect(next.paths).toContain('README.md')
    expect(next.paths.filter((p) => p === 'src/deep/').length).toBe(1)
  })

  it('replaces stale children when a folder is refreshed', () => {
    const tree = mergeSubtree(
      flattenTree(snapshot),
      'src',
      flattenTree(node('src', 'dir', [node('gone.ts', 'file')])),
    )
    expect(tree.paths).toContain('src/gone.ts')
    expect(tree.paths).not.toContain('src/a.ts')
    expect(tree.kinds.has('src/a.ts')).toBe(false)
  })

  it('merging at the root replaces the whole tree', () => {
    const tree = mergeSubtree(flattenTree(snapshot), '', flattenTree(node('root', 'dir', [node('only.ts', 'file')])))
    expect(tree.paths).toEqual(['only.ts'])
  })
})

describe('applyTreeChanges', () => {
  it('adds created files with their missing ancestors', () => {
    const next = applyTreeChanges(emptyTree(), [{ rel: 'a/b/c.ts', kind: 'created', dir: false }])
    expect(next.paths).toEqual(['a/', 'a/b/', 'a/b/c.ts'])
    expect(next.kinds.get('a')).toBe('dir')
    expect(next.kinds.get('a/b/c.ts')).toBe('file')
  })

  it('removes a deleted folder with everything beneath it', () => {
    const tree = flattenTree(snapshot)
    const next = applyTreeChanges(tree, [{ rel: 'src', kind: 'deleted', dir: true }])
    expect(next.paths).toEqual(['README.md', 'node_modules/'])
    expect(next.kinds.has('src/a.ts')).toBe(false)
    expect(next.loaded.has('src')).toBe(false)
  })

  it('returns the same tree when nothing changed', () => {
    const tree = flattenTree(snapshot)
    expect(applyTreeChanges(tree, [{ rel: 'src/a.ts', kind: 'modified', dir: false }])).toBe(tree)
    expect(applyTreeChanges(tree, [{ rel: 'nope.ts', kind: 'deleted', dir: false }])).toBe(tree)
    expect(applyTreeChanges(tree, [])).toBe(tree)
  })

  it('a modify of an unseen path adds it (a creation the watcher folded)', () => {
    const next = applyTreeChanges(flattenTree(snapshot), [{ rel: 'src/new.ts', kind: 'modified', dir: false }])
    expect(next.paths).toContain('src/new.ts')
  })

  it('a file replacing a folder drops the old subtree', () => {
    const next = applyTreeChanges(flattenTree(snapshot), [{ rel: 'src', kind: 'created', dir: false }])
    expect(next.paths).toContain('src')
    expect(next.paths).not.toContain('src/')
    expect(next.paths).not.toContain('src/a.ts')
    expect(next.kinds.get('src')).toBe('file')
  })

  it('leaves paths under an unlisted folder alone until it is fetched', () => {
    const tree = flattenTree(snapshot)
    const next = applyTreeChanges(tree, [{ rel: 'src/deep/new.ts', kind: 'created', dir: false }])
    expect(next).toBe(tree)
    const listed = mergeSubtree(tree, 'src/deep', flattenTree(node('deep', 'dir', [])))
    const after = applyTreeChanges(listed, [{ rel: 'src/deep/new.ts', kind: 'created', dir: false }])
    expect(after.paths).toContain('src/deep/new.ts')
  })

  it('a created folder counts as loaded (it is empty until told otherwise)', () => {
    const next = applyTreeChanges(flattenTree(snapshot), [{ rel: 'src/fresh', kind: 'created', dir: true }])
    expect(isDirLoaded(next, 'src/fresh')).toBe(true)
  })
})

describe('helpers', () => {
  it('changedDirsOf names the parent of every change', () => {
    expect([...changedDirsOf([
      { rel: 'a/b.ts', kind: 'created', dir: false },
      { rel: 'top.ts', kind: 'deleted', dir: false },
    ])]).toEqual(['a', ''])
  })

  it('visibleTruncations hides default-exclude stubs', () => {
    expect(
      visibleTruncations([
        { reason: 'default_exclude', shown: 0, hint: '' },
        { reason: 'max_nodes', shown: 1, hint: '' },
      ]),
    ).toEqual([{ reason: 'max_nodes', shown: 1, hint: '' }])
  })
})
