import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  EDITORS,
  editorById,
  getPreferredEditor,
  setPreferredEditor,
} from './editor-links'

afterEach(() => {
  vi.unstubAllGlobals()
})

function fakeLocalStorage() {
  const store = new Map<string, string>()
  return {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
  }
}

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

describe('preference', () => {
  it('defaults to cursor when nothing is stored', () => {
    vi.stubGlobal('localStorage', fakeLocalStorage())
    expect(getPreferredEditor()).toBe('cursor')
  })

  it('round-trips a stored choice', () => {
    vi.stubGlobal('localStorage', fakeLocalStorage())
    setPreferredEditor('zed')
    expect(getPreferredEditor()).toBe('zed')
  })

  it('resolves unknown stored values to cursor', () => {
    const ls = fakeLocalStorage()
    ls.setItem('iii-preferred-editor', 'emacs')
    vi.stubGlobal('localStorage', ls)
    expect(getPreferredEditor()).toBe('cursor')
  })

  it('is best-effort when storage is unavailable (node env: no localStorage)', () => {
    expect(getPreferredEditor()).toBe('cursor')
    expect(() => setPreferredEditor('vscode')).not.toThrow()
  })
})

it('EDITORS ids are unique and cover the EditorId union', () => {
  expect(EDITORS.map((e) => e.id).sort()).toEqual(['cursor', 'vscode', 'zed'])
})
