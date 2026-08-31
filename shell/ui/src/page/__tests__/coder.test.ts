import { describe, expect, it, vi } from 'vitest'
import {
  coderWriteFile,
  flattenTree,
  joinPath,
  relativeTo,
  type TreeNode,
} from '../coder'

const node = (
  name: string,
  kind: TreeNode['kind'],
  children?: TreeNode[],
): TreeNode => ({ name, kind, size: 0, mtime: 0, children })

describe('flattenTree', () => {
  it('marks directories with a trailing slash so they never collide with their children', () => {
    // Regression: a bare dir path materializes as a FILE in the tree's
    // path store, and its first child then throws "Path collides with an
    // existing file while creating directory".
    const root = node('workers', 'dir', [
      node('code', 'dir', [node('main.rs', 'file')]),
      node('README.md', 'file'),
      node('empty', 'dir', []),
      node('link', 'symlink'),
    ])
    const flat = flattenTree(root)
    expect(flat.paths).toEqual([
      'code/',
      'code/main.rs',
      'README.md',
      'empty/',
      'link',
    ])
  })

  it('keys kinds by the slash-less path (the open-on-select gate)', () => {
    const flat = flattenTree(
      node('r', 'dir', [node('a', 'dir', [node('b.ts', 'file')])]),
    )
    expect(flat.kinds.get('a')).toBe('dir')
    expect(flat.kinds.get('a/b.ts')).toBe('file')
  })

  it('collects truncation hints from any depth', () => {
    const truncated = node('big', 'dir', [])
    truncated.truncated = {
      reason: 'per_folder_limit',
      shown: 10,
      total: 500,
      hint: 'use list-folder',
    }
    const flat = flattenTree(node('r', 'dir', [truncated]))
    expect(flat.truncations).toHaveLength(1)
    expect(flat.truncations[0].reason).toBe('per_folder_limit')
  })
})

describe('path helpers', () => {
  it('joinPath handles the root itself and trailing slashes', () => {
    expect(joinPath('/work', 'a/b.ts')).toBe('/work/a/b.ts')
    expect(joinPath('/work/', 'a')).toBe('/work/a')
    expect(joinPath('/work', '')).toBe('/work')
    expect(joinPath('/work', '.')).toBe('/work')
  })

  it('relativeTo strips the root prefix and tolerates foreign paths', () => {
    expect(relativeTo('/work', '/work/a/b.ts')).toBe('a/b.ts')
    expect(relativeTo('/work', '/work')).toBe('')
    expect(relativeTo('/work', '/elsewhere/x')).toBe('/elsewhere/x')
  })
})

describe('coderWriteFile', () => {
  it('forwards the prior revision and returns the newly written revision', async () => {
    const trigger = vi.fn(async () => ({
      results: [
        {
          path: '/work/a.ts',
          success: true,
          bytes_written: 3,
          revision: 'sha256:new',
        },
      ],
    }))
    const host = {
      iii: { trigger },
    } as unknown as Parameters<typeof coderWriteFile>[0]

    const result = await coderWriteFile(
      host,
      '/work/a.ts',
      'new',
      0o600,
      'sha256:old',
    )

    expect(trigger).toHaveBeenCalledWith('coder::create-file', {
      files: [
        {
          path: '/work/a.ts',
          content: 'new',
          overwrite: true,
          mode: '0600',
          expected_revision: 'sha256:old',
        },
      ],
    })
    expect(result.revision).toBe('sha256:new')
  })
})
