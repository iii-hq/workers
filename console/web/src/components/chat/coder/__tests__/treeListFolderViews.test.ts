import { describe, expect, it } from 'vitest'
import { pageLabel } from '../ListFolderView'
import type { TreeNode } from '../parsers'
import { summariseTree, truncationLabel } from '../TreeView'

describe('summariseTree', () => {
  it('counts the root itself plus all descendants by kind', () => {
    const root: TreeNode = {
      name: 'project',
      kind: 'dir',
      size: 0,
      mtime: 0,
      non_accessible: false,
      children: [
        {
          name: 'src',
          kind: 'dir',
          size: 0,
          mtime: 0,
          non_accessible: false,
          children: [
            {
              name: 'main.rs',
              kind: 'file',
              size: 120,
              mtime: 0,
              non_accessible: false,
            },
            {
              name: 'link',
              kind: 'symlink',
              size: 0,
              mtime: 0,
              non_accessible: false,
            },
          ],
        },
        {
          name: 'README.md',
          kind: 'file',
          size: 42,
          mtime: 0,
          non_accessible: false,
        },
      ],
    }
    // Symlinks count in neither bucket.
    expect(summariseTree(root)).toEqual({ dirs: 2, files: 2, truncated: 0 })
  })

  it('tallies truncation stubs across the whole tree', () => {
    const root: TreeNode = {
      name: 'project',
      kind: 'dir',
      size: 0,
      mtime: 0,
      non_accessible: false,
      truncated: {
        reason: 'per_folder_limit',
        shown: 1,
        total: 9,
        hint: 'use coder::list-folder',
      },
      children: [
        {
          name: 'node_modules',
          kind: 'dir',
          size: 0,
          mtime: 0,
          non_accessible: false,
          truncated: {
            reason: 'default_exclude',
            shown: 0,
            hint: 'pass use_default_excludes: false',
          },
        },
      ],
    }
    expect(summariseTree(root)).toEqual({ dirs: 2, files: 0, truncated: 2 })
  })

  it('handles a single-file root (no children on the wire)', () => {
    const root: TreeNode = {
      name: 'main.rs',
      kind: 'file',
      size: 120,
      mtime: 0,
      non_accessible: false,
    }
    expect(summariseTree(root)).toEqual({ dirs: 0, files: 1, truncated: 0 })
  })
})

describe('truncationLabel', () => {
  it('shows shown/total for per_folder_limit (the only reason with total)', () => {
    expect(
      truncationLabel({
        reason: 'per_folder_limit',
        shown: 50,
        total: 120,
        hint: 'use coder::list-folder',
      }),
    ).toBe('50/120 children shown — paginate with list-folder')
  })

  it('tolerates a per_folder_limit stub missing total', () => {
    expect(
      truncationLabel({
        reason: 'per_folder_limit',
        shown: 50,
        hint: 'use coder::list-folder',
      }),
    ).toBe('50 children shown — paginate with list-folder')
  })

  it('says "not explored" for max_depth (no total on depth cuts)', () => {
    expect(
      truncationLabel({
        reason: 'max_depth',
        shown: 0,
        hint: 'increase max_depth',
      }),
    ).toBe('max depth — subtree not explored')
  })

  it('marks default_exclude stubs as not descended', () => {
    expect(
      truncationLabel({
        reason: 'default_exclude',
        shown: 0,
        hint: 'pass use_default_excludes: false',
      }),
    ).toBe('excluded by default — not descended')
  })

  it('renders unknown reasons verbatim (forward tolerance)', () => {
    expect(
      truncationLabel({ reason: 'budget_exhausted', shown: 3, hint: 'retry' }),
    ).toBe('budget_exhausted')
  })
})

describe('pageLabel', () => {
  it('derives total pages from total entries and effective page size', () => {
    expect(pageLabel(2, 50, 120)).toBe('2/3')
  })

  it('handles exact division', () => {
    expect(pageLabel(1, 50, 100)).toBe('1/2')
  })

  it('clamps an empty folder to one page', () => {
    expect(pageLabel(1, 50, 0)).toBe('1/1')
  })

  it('falls back to the bare page when page_size is 0', () => {
    expect(pageLabel(1, 0, 10)).toBe('1')
  })
})
