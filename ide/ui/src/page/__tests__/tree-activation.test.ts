import { describe, expect, it } from 'vitest'
import {
  filePathFromTreeEvent,
  reactivateSelectedFile,
  shouldActivateTreeSelection,
  treeItemFromEvent,
} from '../tree-activation'

describe('shouldActivateTreeSelection', () => {
  it('suppresses controlled selection while allowing a new user selection', () => {
    expect(shouldActivateTreeSelection('src/app.ts', 'src/app.ts')).toBe(false)
    expect(shouldActivateTreeSelection('src/app.ts', 'README.md')).toBe(true)
    expect(shouldActivateTreeSelection(null, 'README.md')).toBe(false)
  })
})

describe('filePathFromTreeEvent', () => {
  it('finds the file row in a React event native composed path', () => {
    const event = {
      nativeEvent: {
        composedPath: () => [{}, { dataset: { itemPath: 'src/page/FilesTab.tsx', itemType: 'file' } }, {}],
      },
    }

    expect(filePathFromTreeEvent(event)).toBe('src/page/FilesTab.tsx')
  })

  it('also reads a native event composed path and takes the nearest row', () => {
    const event = {
      composedPath: () => [
        {},
        { dataset: { itemPath: 'src/page/FilesTab.tsx' } },
        { dataset: { itemPath: 'src/page' } },
      ],
    }

    expect(filePathFromTreeEvent(event)).toBe('src/page/FilesTab.tsx')
  })

  it('ignores non-elements and empty item paths', () => {
    const event = {
      composedPath: () => [null, 'shadow-root', { dataset: {} }, { dataset: { itemPath: '' } }],
    }

    expect(filePathFromTreeEvent(event)).toBeNull()
    expect(filePathFromTreeEvent({})).toBeNull()
  })

  it('ignores directory rows when the tree exposes their item type', () => {
    const event = {
      composedPath: () => [{ dataset: { itemPath: 'src/page', itemType: 'folder' } }],
    }

    expect(filePathFromTreeEvent(event)).toBeNull()
  })

  it('falls back to data attributes when dataset is unavailable', () => {
    const attributes: Record<string, string> = {
      'data-item-path': 'src/page/GitTab.tsx',
      'data-item-type': 'file',
    }
    const event = {
      composedPath: () => [
        {
          getAttribute: (name: string) => attributes[name] ?? null,
        },
      ],
    }

    expect(filePathFromTreeEvent(event)).toBe('src/page/GitTab.tsx')
  })
})

describe('reactivateSelectedFile', () => {
  it('reactivates a selected file when its row is clicked again', () => {
    const activated: string[] = []
    const event = {
      composedPath: () => [{ dataset: { itemPath: 'src/page/FilesTab.tsx' } }],
    }

    const handled = reactivateSelectedFile(event, 'src/page/FilesTab.tsx', (path) => activated.push(path))

    expect(handled).toBe(true)
    expect(activated).toEqual(['src/page/FilesTab.tsx'])
  })

  it('leaves activation to the tree when the click is not on the selected file', () => {
    const activated: string[] = []
    const event = {
      composedPath: () => [{ dataset: { itemPath: 'src/page/GitTab.tsx', itemType: 'file' } }],
    }

    const handled = reactivateSelectedFile(event, 'src/page/FilesTab.tsx', (path) => activated.push(path))

    expect(handled).toBe(false)
    expect(activated).toEqual([])
  })
})

describe('treeItemFromEvent', () => {
  it('reads a folder row as a directory: @pierre/trees marks it `folder`', () => {
    const row = (itemPath: string, itemType: string) => ({
      composedPath: () => [{}, { dataset: { itemPath, itemType } }],
    })
    expect(treeItemFromEvent(row('src/page/', 'folder'))).toEqual({ path: 'src/page/', kind: 'directory' })
    expect(treeItemFromEvent(row('src/page/a.ts', 'file'))).toEqual({ path: 'src/page/a.ts', kind: 'file' })
  })
})
