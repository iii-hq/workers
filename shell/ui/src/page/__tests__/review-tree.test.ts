import { describe, expect, it } from 'vitest'
import type { FlatTree } from '../coder'
import type { GitChange } from '../git'
import { changedParentDirs, changeTreeRows, withReviewChanges } from '../review-tree'

const change = (path: string, status: GitChange['status'] = 'modified'): GitChange => ({
  path,
  status,
  staged: false,
})

describe('changedParentDirs', () => {
  it('returns every changed ancestor once in shallow-first order', () => {
    expect(
      changedParentDirs([change('src/page/FilesTab.tsx'), change('src/page/index.tsx'), change('README.md')]),
    ).toEqual(['src', 'src/page'])
  })
})

describe('withReviewChanges', () => {
  it('keeps deleted files reviewable after they disappear from disk', () => {
    const tree: FlatTree = {
      paths: ['README.md'],
      kinds: new Map([['README.md', 'file']]),
      truncations: [],
    }

    const merged = withReviewChanges(tree, [change('removed/nested/old.ts', 'deleted')])

    expect(merged?.paths).toEqual(['README.md', 'removed/', 'removed/nested/', 'removed/nested/old.ts'])
    expect(merged?.kinds.get('removed')).toBe('dir')
    expect(merged?.kinds.get('removed/nested')).toBe('dir')
    expect(merged?.kinds.get('removed/nested/old.ts')).toBe('file')
  })

  it('does not duplicate files already present in the workspace tree', () => {
    const tree: FlatTree = {
      paths: ['src/', 'src/app.ts'],
      kinds: new Map([
        ['src', 'dir'],
        ['src/app.ts', 'file'],
      ]),
      truncations: [],
    }

    expect(withReviewChanges(tree, [change('src/app.ts')])?.paths).toEqual(tree.paths)
  })

  it('does not collide when a deleted file has become a directory', () => {
    const tree: FlatTree = {
      paths: ['foo/', 'foo/bar.ts'],
      kinds: new Map([
        ['foo', 'dir'],
        ['foo/bar.ts', 'file'],
      ]),
      truncations: [],
    }

    expect(withReviewChanges(tree, [change('foo', 'deleted')])?.paths).toEqual(tree.paths)
  })

  it('does not collide when a deleted directory has become a file', () => {
    const tree: FlatTree = {
      paths: ['foo'],
      kinds: new Map([['foo', 'file']]),
      truncations: [],
    }

    expect(withReviewChanges(tree, [change('foo/bar.ts', 'deleted')])?.paths).toEqual(tree.paths)
  })
})

describe('changeTreeRows', () => {
  it('uses invisible ids for file-to-directory replacements', () => {
    const rows = changeTreeRows([change('foo', 'deleted'), change('foo/bar.ts', 'untracked')])
    expect(rows.map((row) => row.treePath)).toEqual(['foo\u2063', 'foo/bar.ts'])
    expect(rows.map((row) => row.change.path)).toEqual(['foo', 'foo/bar.ts'])
  })
})
