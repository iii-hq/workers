import { describe, expect, it } from 'vitest'
import { markTreeMenuRows } from '../tree-activation'
import { TREE_UNSAFE_CSS } from '../tree-theme'

/** @pierre/trees rows in a shadow root, by path (a folder's ends in /),
    and a root that matches attribute selectors against them. */
function tree(...paths: string[]) {
  const rows = paths.map((itemPath) => {
    const attributes = new Map([
      ['data-type', 'item'],
      ['data-item-path', itemPath],
    ])
    return {
      attributes,
      get dataset(): Record<string, string> {
        return Object.fromEntries(
          [...attributes].map(([name, value]) => [
            name.slice('data-'.length).replace(/-(\w)/g, (_, char: string) => char.toUpperCase()),
            value,
          ]),
        )
      },
      setAttribute(name: string, value: string) {
        attributes.set(name, value)
      },
      removeAttribute(name: string) {
        attributes.delete(name)
      },
    }
  })
  const root = {
    querySelectorAll: (selector: string) => {
      const terms = [...selector.matchAll(/\[([\w-]+)(?:=(['"]?)([^'"\]]*)\2)?\]/g)]
      if (terms.map(([term]) => term).join('') !== selector) throw new Error(`unsupported selector ${selector}`)
      return rows.filter((row) =>
        terms.every(
          ([, name, , value]) =>
            row.attributes.has(name) && (value === undefined || row.attributes.get(name) === value),
        ),
      )
    },
  }
  return {
    rows,
    mark: (path: string | null) => markTreeMenuRows(root as unknown as ParentNode, path),
    /** Each marked row's path and mark. */
    marks: () =>
      Object.fromEntries(
        rows
          .filter((row) => row.attributes.has('data-shui-menu'))
          .map((row) => [row.dataset.itemPath, row.attributes.get('data-shui-menu')]),
      ),
  }
}

describe('markTreeMenuRows', () => {
  it('marks a file as the target and nothing under it, not even a longer name', () => {
    const { rows, mark, marks } = tree('src/', 'src/a.ts', 'src/a.ts.map', 'README.md')
    expect(mark('src/a.ts')).toBe(rows[1])
    expect(marks()).toEqual({ 'src/a.ts': 'target' })
  })

  it("marks a folder's rows as its scope, and not a sibling sharing its prefix", () => {
    for (const sticky of [0, 1]) {
      const { rows, mark, marks } = tree('src/', 'src/', 'src/a.ts', 'src/lib/b.ts', 'src2/c.ts', 'README.md')
      rows[sticky].setAttribute('data-file-tree-sticky-row', 'true')
      // The sticky copy is marked too, but the menu opens under the row in place.
      expect(mark('src/')).toBe(rows[1 - sticky])
      expect(rows[sticky].attributes.get('data-shui-menu')).toBe('target')
      expect(marks()).toEqual({ 'src/': 'target', 'src/a.ts': 'scope', 'src/lib/b.ts': 'scope' })
    }
  })

  it('clears the previous marks, and all of them on null', () => {
    const { mark, marks } = tree('src/', 'src/a.ts', 'README.md')
    mark('src/')
    mark('README.md')
    expect(marks()).toEqual({ 'README.md': 'target' })
    mark('src/')
    expect(mark(null)).toBeNull()
    expect(marks()).toEqual({})
  })

  it('sets exactly the marks tree-theme styles', () => {
    const { rows, mark } = tree('src/', 'src/a.ts')
    mark('src/')
    const set = rows
      // Past the two every row starts with.
      .flatMap((row) => [...row.attributes].slice(2).map(([name, value]) => `${name}=${value}`))
      .sort()
    const styled = [...TREE_UNSAFE_CSS.matchAll(/\[(data-shui-menu)='([^']+)'\]/g)].map(
      ([, name, value]) => `${name}=${value}`,
    )
    expect(set).toEqual([...new Set(styled)].sort())
  })
})
