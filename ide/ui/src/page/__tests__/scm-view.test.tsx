import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ChangeEntries } from '../ChangeEntries'
import type { GitComparisonEntry } from '../git'
import { buildChangeTree, readScmViewMode, writeScmViewMode } from '../scm-view'

function entry(path: string, status: GitComparisonEntry['status'] = 'modified'): GitComparisonEntry {
  return { path, status, staged: false, x: ' ', y: 'M', before: { kind: 'index', path }, after: { kind: 'worktree', path } }
}

afterEach(() => vi.unstubAllGlobals())

describe('SCM view', () => {
  it('groups nested paths, sorts folders first, and retains comparison metadata', () => {
    const renamed = { ...entry('src/deep/new.ts', 'renamed'), renameFrom: 'old.ts' }
    const deleted = entry('src/gone.ts', 'deleted')
    const rootFile = entry('README.md')
    const entries = [renamed, rootFile, deleted, entry('assets/icon.svg'), entry('src/a.ts')]
    const original = [...entries]
    const tree = buildChangeTree(entries)
    expect(tree.directories.map((dir) => dir.path)).toEqual(['assets', 'src'])
    expect(tree.entries).toEqual([rootFile])
    const src = tree.directories[1]
    expect(src.entries.map((file) => file.path)).toEqual(['src/a.ts', 'src/gone.ts'])
    expect(src.entries[1]).toBe(deleted)
    expect(src.directories[0].path).toBe('src/deep')
    expect(src.directories[0].entries[0]).toBe(renamed)
    expect(entries).toEqual(original)
  })

  it('handles empty sections and repeated basenames in different directories', () => {
    expect(buildChangeTree([])).toEqual({ name: '', path: '', directories: [], entries: [] })
    const tree = buildChangeTree([entry('a/index.ts'), entry('b/index.ts'), entry('index.ts')])
    expect(tree.directories).toHaveLength(2)
    expect(tree.entries).toHaveLength(1)
  })

  it('does not interpret special folder names as object properties', () => {
    expect(buildChangeTree([entry('__proto__/constructor/file.ts')]).directories[0].directories[0].entries).toHaveLength(1)
  })

  it('renders the flat list in its original order without folder controls', () => {
    const html = renderToStaticMarkup(<ChangeEntries
      entries={[entry('z/b.ts'), entry('a.ts')]}
      mode="list"
      renderEntry={(file, depth) => <button key={file.path} data-depth={depth}>{file.path}</button>}
    />)
    expect(html).not.toContain('aria-expanded')
    expect(html.indexOf('z/b.ts')).toBeLessThan(html.indexOf('a.ts'))
    expect(html.match(/data-depth="0"/g)).toHaveLength(2)
  })

  it('renders expanded, keyboard-operable folder buttons and indented files', () => {
    const html = renderToStaticMarkup(<ChangeEntries
      entries={[entry('src/nested/a.ts'), entry('README.md')]}
      mode="tree"
      renderEntry={(file, depth) => <button key={file.path} data-depth={depth}>{file.path}</button>}
    />)
    expect(html.match(/aria-expanded="true"/g)).toHaveLength(2)
    expect(html).toContain('title="src/nested"')
    expect(html).toContain('data-depth="2">src/nested/a.ts')
    expect(html).toContain('data-depth="0">README.md')
  })

  it('defaults to list and persists both view choices', () => {
    const values = new Map<string, string>()
    vi.stubGlobal('window', { localStorage: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    } })
    expect(readScmViewMode()).toBe('list')
    writeScmViewMode('tree')
    expect(readScmViewMode()).toBe('tree')
    writeScmViewMode('list')
    expect(readScmViewMode()).toBe('list')
    for (const key of values.keys()) values.set(key, 'invalid')
    expect(readScmViewMode()).toBe('list')
  })

  it('survives unavailable storage and server rendering', () => {
    vi.stubGlobal('window', undefined)
    expect(readScmViewMode()).toBe('list')
    expect(() => writeScmViewMode('tree')).not.toThrow()
    vi.stubGlobal('window', { get localStorage() { throw new Error('blocked') } })
    expect(readScmViewMode()).toBe('list')
    expect(() => writeScmViewMode('tree')).not.toThrow()
  })
})
