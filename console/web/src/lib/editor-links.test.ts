import { describe, expect, it } from 'vitest'
import { EDITORS, editorById } from './editor-links'

describe('buildUrl', () => {
  it('builds the three schemes around the same file path', () => {
    const path = '/home/anderson/project/mod.rs'
    expect(editorById('cursor').buildUrl(path)).toBe(
      'cursor://file/home/anderson/project/mod.rs',
    )
    expect(editorById('vscode').buildUrl(path)).toBe(
      'vscode://file/home/anderson/project/mod.rs',
    )
    expect(editorById('zed').buildUrl(path)).toBe(
      'zed://file/home/anderson/project/mod.rs',
    )
  })

  it('appends a positive integer line anchor', () => {
    expect(editorById('cursor').buildUrl('/a/b.ts', 42)).toBe(
      'cursor://file/a/b.ts:42',
    )
  })

  it('omits non-positive or fractional line anchors', () => {
    expect(editorById('cursor').buildUrl('/a/b.ts', 0)).toBe(
      'cursor://file/a/b.ts',
    )
    expect(editorById('cursor').buildUrl('/a/b.ts', -3)).toBe(
      'cursor://file/a/b.ts',
    )
    expect(editorById('cursor').buildUrl('/a/b.ts', 1.5)).toBe(
      'cursor://file/a/b.ts',
    )
  })

  it('percent-encodes spaces while preserving slashes', () => {
    expect(editorById('zed').buildUrl('/tmp/my file.txt', 3)).toBe(
      'zed://file/tmp/my%20file.txt:3',
    )
  })
})

it('EDITORS ids are unique and cover the EditorId union', () => {
  expect(EDITORS.map((e) => e.id).sort()).toEqual(['cursor', 'vscode', 'zed'])
})
