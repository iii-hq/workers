import { describe, expect, it, vi } from 'vitest'
import {
  coderDelete,
  coderWriteFile,
  flattenTree,
  joinPath,
  relativeTo,
  type TreeNode,
} from '../coder'
import { deleteAfterRefusal, deleteEntry, isProtectedSubtreeError } from '../file-actions'

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

  it("lists dot entries but leaves git's own folder out", () => {
    const flat = flattenTree(
      node('r', 'dir', [
        node('.git', 'dir', []),
        node('.github', 'dir', []),
        node('.gitignore', 'file'),
      ]),
    )
    expect(flat.paths).toEqual(['.github/', '.gitignore'])
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

describe('coderDelete', () => {
  it('asks to remove protected files only when the user confirmed it', async () => {
    const trigger = vi.fn(async () => ({ results: [] }))
    const host = { iii: { trigger } } as unknown as Parameters<typeof coderDelete>[0]
    await coderDelete(host, ['/r/d'], true)
    await coderDelete(host, ['/r/d'], true, true)
    expect(trigger.mock.calls.map((call) => (call as unknown[])[1])).toEqual([
      { paths: ['/r/d'], recursive: true },
      { paths: ['/r/d'], recursive: true, include_protected: true },
    ])
  })

  it("tells the worker's protected-subtree refusal from other failures", () => {
    expect(
      isProtectedSubtreeError('/r/test-worker: subtree contains non-accessible entries; refusing recursive delete.'),
    ).toBe(true)
    expect(isProtectedSubtreeError('/r/x: not found or not accessible.')).toBe(false)
  })
})

describe('deleteEntry', () => {
  it('deletes a folder recursively, its protected entries only when confirmed', async () => {
    const trigger = vi.fn(async () => ({ results: [{ path: '/r/pkg', success: true, removed: true }] }))
    const host = { iii: { trigger } } as unknown as Parameters<typeof deleteEntry>[0]
    await deleteEntry(host, '/r', 'pkg', true)
    await deleteEntry(host, '/r', 'pkg', true, true)
    expect(trigger.mock.calls).toEqual([
      ['coder::delete-file', { paths: ['/r/pkg'], recursive: true }],
      ['coder::delete-file', { paths: ['/r/pkg'], recursive: true, include_protected: true }],
    ])
  })

  it("throws the worker's error for a path it could not delete", async () => {
    const trigger = vi.fn(async () => ({ results: [{ success: false, error: { message: 'x' } }] }))
    const host = { iii: { trigger } } as unknown as Parameters<typeof deleteEntry>[0]
    await expect(deleteEntry(host, '/r', 'pkg', true)).rejects.toThrow(/^x$/)
  })
})

describe('deleteAfterRefusal', () => {
  const refused = '/r/pkg: subtree contains non-accessible entries; refusing recursive delete.'

  it('asks again, naming the protected files, when a folder delete was refused for them', () => {
    expect(deleteAfterRefusal({ path: 'pkg', isDir: true }, refused)).toEqual({
      path: 'pkg',
      isDir: true,
      protectedInside: true,
    })
  })

  it('never asks a third time, nor for a file or another failure', () => {
    expect(deleteAfterRefusal({ path: 'pkg', isDir: true, protectedInside: true }, refused)).toBeNull()
    expect(deleteAfterRefusal({ path: 'a.pem', isDir: false }, refused)).toBeNull()
    expect(deleteAfterRefusal({ path: 'pkg', isDir: true }, 'permission denied')).toBeNull()
  })
})
