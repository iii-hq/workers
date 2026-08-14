import { describe, expect, it, vi } from 'vitest'
import {
  classifyWorkspaceBaselinePath,
  captureWorkspaceBaseline,
} from '../baseline'
import type { TreeNode } from '../coder'
import { normalizeLiveReviewEvent } from '../live-review'
import { mergeGitReviewEntries, mergeReviewEntry } from '../review'

function file(name: string): TreeNode {
  return { name, kind: 'file', size: 7, mtime: 1 }
}

function workspace(children: TreeNode[], truncated: boolean | string = false): TreeNode {
  return {
    name: 'repo',
    kind: 'dir',
    size: 0,
    mtime: 1,
    children,
    truncated: truncated
      ? {
          reason: truncated === true ? 'per_folder_limit' : truncated,
          shown: children.length,
          total: children.length + 1,
          hint: 'raise the limit',
        }
      : null,
  }
}

function hostFor(root: TreeNode) {
  const trigger = vi.fn(async (functionId: string, _input: unknown) => {
    if (functionId === 'coder::tree') return { path: '/repo', root }
    if (functionId === 'coder::read-file') {
      return {
        results: [
          {
            path: '/repo/visible.ts',
            success: true,
            content: 'before\n',
            is_utf8: true,
            more_lines: false,
          },
        ],
      }
    }
    throw new Error(`unexpected function ${functionId}`)
  })
  return {
    host: { iii: { trigger } } as unknown as Parameters<typeof captureWorkspaceBaseline>[0],
    trigger,
  }
}

describe('captureWorkspaceBaseline', () => {
  it('publishes a complete inventory only when the tree has no truncated nodes', async () => {
    const { host, trigger } = hostFor(workspace([file('visible.ts')]))
    const baseline = await captureWorkspaceBaseline(
      host,
      '/repo',
      () => true,
    )

    expect(trigger).toHaveBeenCalledWith(
      'coder::tree',
      expect.objectContaining({ include_hidden: true, use_default_excludes: true }),
    )
    expect(baseline.complete).toBe(true)
    expect(baseline.contents.get('visible.ts')).toBe('before\n')
    expect(classifyWorkspaceBaselinePath(baseline, 'visible.ts')).toEqual({
      priorKind: 'file',
      exact: true,
    })
    expect(classifyWorkspaceBaselinePath(baseline, 'new.ts')).toEqual({
      priorKind: null,
      exact: true,
    })
  })

  it('marks truncated inventory incomplete and fails unknown paths closed', async () => {
    const baseline = await captureWorkspaceBaseline(
      hostFor(workspace([file('visible.ts')], true)).host,
      '/repo',
      () => true,
    )

    expect(baseline.complete).toBe(false)
    // Known entries remain useful, but an omitted existing file and a truly
    // new file are indistinguishable. Both must avoid an exact empty baseline.
    expect(classifyWorkspaceBaselinePath(baseline, 'visible.ts')).toEqual({
      priorKind: 'file',
      exact: true,
    })
    expect(classifyWorkspaceBaselinePath(baseline, 'unknown.ts')).toEqual({
      priorKind: 'file',
      exact: false,
    })
  })

  it.each(['max_nodes', 'max_depth', 'future_truncation_reason'])(
    'fails closed for %s truncation',
    async (reason) => {
      const baseline = await captureWorkspaceBaseline(
        hostFor(workspace([file('visible.ts')], reason)).host,
        '/repo',
        () => true,
      )

      expect(baseline.complete).toBe(false)
      expect(classifyWorkspaceBaselinePath(baseline, 'unknown.ts')).toEqual({
        priorKind: 'file',
        exact: false,
      })
    },
  )

  it('treats intentional default-exclude stubs as complete for new source files', async () => {
    const gitDir: TreeNode = {
      name: '.git',
      kind: 'dir',
      size: 0,
      mtime: 1,
      children: [],
      truncated: {
        reason: 'default_exclude',
        shown: 0,
        hint: 'excluded noise folder',
      },
    }
    const srcDir: TreeNode = {
      name: 'src',
      kind: 'dir',
      size: 0,
      mtime: 1,
      children: [],
    }
    const baseline = await captureWorkspaceBaseline(
      hostFor(workspace([gitDir, srcDir])).host,
      '/repo',
      (path) => !path.split('/').includes('.git'),
    )

    expect(baseline.complete).toBe(true)
    const classification = classifyWorkspaceBaselinePath(baseline, 'src/new.ts')
    expect(classification).toEqual({ priorKind: null, exact: true })
    const decision = normalizeLiveReviewEvent({
      path: 'src/new.ts',
      rawKind: 'created',
      priorKind: classification.priorKind,
      priorBaseline: undefined,
      existsNow: true,
    })
    expect(decision).toEqual({
      action: 'created',
      path: 'src/new.ts',
      baseline: '',
    })
    if (!('baseline' in decision)) throw new Error('expected a reviewable file decision')
    const entry = mergeReviewEntry(
      new Map(),
      'src/new.ts',
      decision.action,
      decision.baseline,
      { path: 'src/new.ts', status: 'untracked', staged: false },
    )
    expect(entry.get('src/new.ts')).toMatchObject({
      baseline: '',
      change: { status: 'untracked' },
    })
  })

  it('keeps a later source sibling exact when installed-repo noise is heavily excluded', async () => {
    const excludedDir = (name: string, total: number): TreeNode => ({
      name,
      kind: 'dir',
      size: 0,
      mtime: 1,
      children: [],
      truncated: {
        reason: 'default_exclude',
        shown: 0,
        total,
        hint: 'configured default exclude',
      },
    })
    const src: TreeNode = {
      name: 'src',
      kind: 'dir',
      size: 0,
      mtime: 1,
      children: [file('app.ts')],
    }
    const baseline = await captureWorkspaceBaseline(
      hostFor(workspace([excludedDir('.git', 8_000), excludedDir('node_modules', 50_000), src])).host,
      '/repo',
      (path) => {
        const segments = path.split('/')
        return !segments.includes('.git') && !segments.includes('node_modules')
      },
    )

    expect(baseline.complete).toBe(true)
    expect(baseline.kinds.get('src/app.ts')).toBe('file')
    expect(classifyWorkspaceBaselinePath(baseline, 'src/new.ts')).toEqual({
      priorKind: null,
      exact: true,
    })
  })

  it.each(['.venv', 'generated-sources'])(
    'fails closed when a reviewable %s directory is default-excluded',
    async (directory) => {
      const excluded: TreeNode = {
        name: directory,
        kind: 'dir',
        size: 0,
        mtime: 1,
        children: [],
        truncated: {
          reason: 'default_exclude',
          shown: 0,
          hint: 'configured default exclude',
        },
      }
      const baseline = await captureWorkspaceBaseline(
        hostFor(workspace([excluded])).host,
        '/repo',
        (path) => !path.split('/').includes('.git'),
      )

      expect(baseline.complete).toBe(false)
      expect(classifyWorkspaceBaselinePath(baseline, `${directory}/existing.ts`)).toEqual({
        priorKind: 'file',
        exact: false,
      })
    },
  )

  it('fails closed when a custom generated-file glob omits a reviewable child', async () => {
    const src: TreeNode = {
      name: 'src',
      kind: 'dir',
      size: 0,
      mtime: 1,
      children: [file('app.ts')],
      truncated: {
        reason: 'default_exclude',
        shown: 1,
        hint: '1 non-directory entry was omitted by default_exclude_globs',
      },
    }
    const baseline = await captureWorkspaceBaseline(
      hostFor(workspace([src])).host,
      '/repo',
      (path) => !path.split('/').includes('.git') && !path.split('/').includes('node_modules'),
    )

    expect(baseline.complete).toBe(false)
    expect(classifyWorkspaceBaselinePath(baseline, 'src/schema.generated.ts')).toEqual({
      priorKind: 'file',
      exact: false,
    })
  })

  it('keeps an unknown path unavailable even when Git later identifies it as untracked', async () => {
    const baseline = await captureWorkspaceBaseline(
      hostFor(workspace([file('visible.ts')], true)).host,
      '/repo',
      () => true,
    )
    const classification = classifyWorkspaceBaselinePath(baseline, 'unknown.ts')
    const decision = normalizeLiveReviewEvent({
      path: 'unknown.ts',
      rawKind: 'created',
      priorKind: classification.priorKind,
      priorBaseline: undefined,
      existsNow: true,
    })
    expect(decision).toEqual({
      action: 'modified',
      path: 'unknown.ts',
      baseline: undefined,
    })
    if (!('baseline' in decision)) throw new Error('expected a reviewable file decision')

    const gitChange = {
      path: 'unknown.ts',
      status: 'untracked' as const,
      staged: false,
    }
    const baselineUnavailable =
      classification.priorKind === 'file' && !baseline.contents.has('unknown.ts')
    const initial = mergeReviewEntry(
      new Map(),
      'unknown.ts',
      decision.action,
      decision.baseline,
      // The index integration withholds this fallback when the captured
      // inventory says an existing body could have been omitted.
      baselineUnavailable ? undefined : gitChange,
    )
    const enriched = mergeGitReviewEntries(initial, [gitChange], false)
    expect(enriched.get('unknown.ts')).toMatchObject({
      baseline: null,
      change: { status: 'untracked' },
    })
  })
})
