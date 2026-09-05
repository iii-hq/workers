import { describe, expect, it } from 'vitest'
import {
  diffPanelRequest,
  isFileChangesResponse,
  summarizeFileChanges,
} from '../file-changes'

describe('summarizeFileChanges', () => {
  it('summarizes created files and line additions', () => {
    expect(
      summarizeFileChanges(
        'coder::create-file',
        {
          files: [
            { path: 'src/a.ts', content: 'one\ntwo\n' },
            { path: 'src/b.ts', content: 'one', overwrite: true },
          ],
        },
        {
          results: [
            {
              path: '/repo/src/a.ts',
              success: true,
              bytes_written: 8,
              change_id: 'change-a',
            },
            { path: '/repo/src/b.ts', success: true, bytes_written: 3 },
          ],
        },
      ),
    ).toEqual({
      action: 'created',
      rows: [
        {
          path: 'src/a.ts',
          absolutePath: '/repo/src/a.ts',
          changeId: 'change-a',
          status: 'created',
          additions: 2,
        },
        {
          path: 'src/b.ts',
          absolutePath: '/repo/src/b.ts',
          status: 'updated',
          additions: 1,
        },
      ],
    })
  })

  it('counts line edits and leaves regex replacement counts unknown', () => {
    expect(
      summarizeFileChanges('coder::update-file', {
        files: [
          {
            path: 'src/a.ts',
            ops: [
              { op: 'insert', at_line: 2, content: 'a\nb' },
              { op: 'remove', from_line: 5, to_line: 7 },
              {
                op: 'update_lines',
                from_line: 10,
                to_line: 11,
                content: 'c\n',
              },
            ],
          },
          {
            path: 'src/b.ts',
            ops: [{ op: 'replace', pattern: 'old', replacement: 'new' }],
          },
        ],
      }),
    ).toEqual({
      action: 'updated',
      rows: [
        {
          path: 'src/a.ts',
          status: 'updated',
          additions: 3,
          deletions: 5,
        },
        { path: 'src/b.ts', status: 'updated' },
      ],
    })
  })

  it('reports failed and unchanged deletions from their result entries', () => {
    expect(
      summarizeFileChanges(
        'coder::delete-file',
        { paths: ['old.ts', 'missing.ts', 'protected.ts'] },
        {
          results: [
            { path: '/repo/old.ts', success: true, removed: true },
            { path: '/repo/missing.ts', success: true, removed: false },
            { path: 'protected.ts', success: false, removed: false },
          ],
        },
      ),
    ).toEqual({
      action: 'deleted',
      rows: [
        {
          path: 'old.ts',
          absolutePath: '/repo/old.ts',
          status: 'deleted',
        },
        {
          path: 'missing.ts',
          absolutePath: '/repo/missing.ts',
          status: 'unchanged',
        },
        { path: 'protected.ts', absolutePath: 'protected.ts', status: 'failed' },
      ],
    })
  })

  it('returns null for unrelated functions and malformed inputs', () => {
    expect(summarizeFileChanges('shell::exec', {})).toBeNull()
    expect(summarizeFileChanges('coder::create-file', { files: null })).toBeNull()
  })

  it('distinguishes coder results from gate or transport errors', () => {
    expect(
      isFileChangesResponse({
        results: [{ path: 'a.ts', success: false, bytes_written: 0 }],
      }),
    ).toBe(true)
    expect(
      isFileChangesResponse({
        error: { kind: 'function_error', message: 'denied' },
      }),
    ).toBe(false)
  })

  it('builds shell panel requests for exact diffs and file editing', () => {
    const row = {
      path: 'src/a.ts',
      absolutePath: '/repo/src/a.ts',
      changeId: 'snapshot-1',
      status: 'updated' as const,
    }
    expect(diffPanelRequest(row)).toEqual({
      pageId: 'shell',
      context: {
        type: 'change-diff',
        changeId: 'snapshot-1',
        path: '/repo/src/a.ts',
        canViewFile: true,
      },
    })
    expect(diffPanelRequest({ ...row, status: 'deleted' })).toMatchObject({
      context: { canViewFile: false },
    })
  })
})
