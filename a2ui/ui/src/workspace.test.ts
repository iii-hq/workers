import type { Host } from '@iii-dev/console-ui'
import { describe, expect, it, vi } from 'vitest'
import type { CodeExport } from './types'
import { projectDirectory, writeCodeBundle } from './workspace'

describe('A2UI workspace handoff', () => {
  it('writes every generated React file below a revisioned workspace directory', async () => {
    const trigger = vi.fn(async (_functionId: string, payload: { files: Array<{ path: string; content: string }> }) => ({
      results: payload.files.map((file) => ({ path: file.path, success: true })),
    }))
    const bundle = fixtureBundle()
    const result = await writeCodeBundle(mockHost(trigger), '/workspace/project', bundle, 4)

    expect(trigger).toHaveBeenCalledWith('coder::create-file', {
      files: [
        {
          path: '/workspace/project/generated/a2ui/incident-review-r4/package.json',
          content: '{}\n',
          parents: true,
          overwrite: false,
        },
        {
          path: '/workspace/project/generated/a2ui/incident-review-r4/src/main.tsx',
          content: 'export {}\n',
          parents: true,
          overwrite: false,
        },
      ],
    })
    expect(result).toEqual({
      directory: '/workspace/project/generated/a2ui/incident-review-r4',
      files: [
        '/workspace/project/generated/a2ui/incident-review-r4/package.json',
        '/workspace/project/generated/a2ui/incident-review-r4/src/main.tsx',
      ],
      written: 2,
      existing: 0,
    })
  })

  it('preserves previously materialized files rather than overwriting edits', async () => {
    const trigger = vi.fn(async () => ({
      results: [
        {
          path: '/workspace/generated/a2ui/incident-review-r4/package.json',
          success: false,
          error: { code: 'C213', message: 'already exists' },
        },
        {
          path: '/workspace/generated/a2ui/incident-review-r4/src/main.tsx',
          success: true,
        },
      ],
    }))
    const result = await writeCodeBundle(mockHost(trigger), '/workspace', fixtureBundle(), 4)
    expect(result).toMatchObject({ written: 1, existing: 1 })
  })

  it('rejects unsafe generated paths before calling Shell', async () => {
    const trigger = vi.fn()
    const bundle = fixtureBundle()
    bundle.files[1]!.path = '../outside.tsx'
    await expect(writeCodeBundle(mockHost(trigger), '/workspace', bundle, 4)).rejects.toThrow(
      'unsafe path',
    )
    expect(trigger).not.toHaveBeenCalled()
  })

  it('constructs a visible project path for POSIX and Windows workspaces', () => {
    expect(projectDirectory('/workspace/', 'Incident Review', 7)).toBe(
      '/workspace/generated/a2ui/Incident-Review-r7',
    )
    expect(projectDirectory('C:\\work\\repo\\', 'Incident Review', 7)).toBe(
      'C:\\work\\repo\\generated\\a2ui\\Incident-Review-r7',
    )
  })
})

function fixtureBundle(): CodeExport {
  return {
    format: 'react',
    surface_id: 'incident-review',
    files: [
      { path: 'package.json', content: '{}\n' },
      { path: 'src/main.tsx', content: 'export {}\n' },
    ],
  }
}

function mockHost(trigger: ReturnType<typeof vi.fn>): Host {
  return { iii: { trigger } } as unknown as Host
}
