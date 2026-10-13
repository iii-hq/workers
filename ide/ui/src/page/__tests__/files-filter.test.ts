import { FileTree, type FileTreeDirectoryHandle } from '@pierre/trees'
import { describe, expect, it } from 'vitest'
import { expandedDirectoryPaths } from '../tree-expansion'

// The explorer stops reporting expansion while its filter is open and
// trusts the model to put the user's expansion back when it closes.
describe('explorer filter expansion', () => {
  it('opens matching stubs only while the search is open, then restores the user expansion', () => {
    const model = new FileTree({
      paths: ['src/', 'src/a.ts', 'vendor/', 'docs/', 'docs/x.md'],
      fileTreeSearchMode: 'hide-non-matches',
      search: false,
      initialExpandedPaths: ['docs/'],
    })
    const dirs = ['src', 'vendor', 'docs']
    const seen: { searching: boolean; open: string[] }[] = []
    model.subscribe(() => seen.push({ searching: model.isSearchOpen(), open: expandedDirectoryPaths(model, dirs) }))

    model.setSearch('vendor')
    model.setSearch(null)

    expect(seen).toEqual([
      { searching: true, open: ['vendor'] },
      { searching: false, open: ['docs'] },
    ])
  })

  // So no folder the user opens during a filter goes unlisted: the model
  // holds the expansion to the matches until the filter closes.
  it('undoes a folder opened or closed while the search is open', () => {
    const model = new FileTree({
      paths: ['src/', 'src/a.ts', 'vendor/', 'docs/', 'docs/x.md'],
      fileTreeSearchMode: 'hide-non-matches',
      search: false,
    })
    const dirs = ['src', 'vendor', 'docs']
    const handle = (path: string) => model.getItem(`${path}/`) as FileTreeDirectoryHandle

    model.setSearch('a.ts')
    handle('docs').expand()
    handle('src').collapse()

    expect(expandedDirectoryPaths(model, dirs)).toEqual(['src'])
  })
})
