import type { PanelOpenRequest } from '@iii-dev/console-ui'
import { z } from 'zod'

const CREATE_ID = 'coder::create-file'
const UPDATE_ID = 'coder::update-file'
const DELETE_ID = 'coder::delete-file'

const resultSchema = z.object({
  path: z.string(),
  success: z.boolean(),
  bytes_written: z.number().optional(),
  removed: z.boolean().optional(),
  applied: z.number().optional(),
  change_id: z.string().optional(),
})

const responseSchema = z.object({ results: z.array(resultSchema) })

export function isFileChangesResponse(output: unknown): boolean {
  return responseSchema.safeParse(output).success
}

const createRequestSchema = z.object({
  files: z.array(
    z.object({
      path: z.string(),
      content: z.string(),
      overwrite: z.boolean().optional(),
    }),
  ),
})

const updateOpSchema = z.discriminatedUnion('op', [
  z.object({ op: z.literal('insert'), content: z.string() }),
  z.object({
    op: z.literal('remove'),
    from_line: z.number(),
    to_line: z.number(),
  }),
  z.object({
    op: z.literal('update_lines'),
    from_line: z.number(),
    to_line: z.number(),
    content: z.string(),
  }),
  z.object({ op: z.literal('replace') }),
])

const updateRequestSchema = z.object({
  files: z.array(
    z.object({
      path: z.string(),
      ops: z.array(updateOpSchema),
    }),
  ),
})

const deleteRequestSchema = z.object({ paths: z.array(z.string()) })

export type FileChangeStatus = 'created' | 'updated' | 'deleted' | 'unchanged' | 'failed'

export interface FileChangeRow {
  path: string
  /** Canonical worker-resolved path, available after a successful call. */
  absolutePath?: string
  /** Exact before/after snapshot stored by the shell worker. */
  changeId?: string
  status: FileChangeStatus
  additions?: number
  deletions?: number
}

export interface FileChangesSummary {
  action: 'created' | 'updated' | 'deleted'
  rows: FileChangeRow[]
}

export function diffPanelRequest(row: FileChangeRow): PanelOpenRequest {
  return {
    pageId: 'shell',
    context: {
      type: 'change-diff',
      changeId: row.changeId ?? '',
      path: row.absolutePath ?? row.path,
      canViewFile: row.status === 'created' || row.status === 'updated',
    },
  }
}

export function filePanelRequest(row: FileChangeRow): PanelOpenRequest {
  return {
    pageId: 'shell',
    context: { type: 'file', path: row.absolutePath ?? row.path },
  }
}

function countLines(content: string): number {
  if (content.length === 0) return 0
  const lines = content.split('\n')
  return lines.at(-1) === '' ? lines.length - 1 : lines.length
}

function resultAt(output: unknown, index: number) {
  const parsed = responseSchema.safeParse(output)
  return parsed.success ? parsed.data.results[index] : undefined
}

function resultContext(output: unknown, index: number) {
  const result = resultAt(output, index)
  return {
    ...(result?.path ? { absolutePath: result.path } : {}),
    ...(result?.change_id ? { changeId: result.change_id } : {}),
  }
}

export function summarizeFileChanges(functionId: string, input: unknown, output?: unknown): FileChangesSummary | null {
  if (functionId === CREATE_ID) {
    const req = createRequestSchema.safeParse(input)
    if (!req.success) return null
    return {
      action: 'created',
      rows: req.data.files.map((file, index) => {
        const result = resultAt(output, index)
        return {
          path: file.path,
          ...resultContext(output, index),
          status: result && !result.success ? 'failed' : file.overwrite ? 'updated' : 'created',
          additions: countLines(file.content),
        }
      }),
    }
  }

  if (functionId === UPDATE_ID) {
    const req = updateRequestSchema.safeParse(input)
    if (!req.success) return null
    return {
      action: 'updated',
      rows: req.data.files.map((file, index) => {
        const result = resultAt(output, index)
        let additions = 0
        let deletions = 0
        let countsKnown = true
        for (const op of file.ops) {
          if (op.op === 'insert') additions += countLines(op.content)
          else if (op.op === 'remove') {
            deletions += op.to_line - op.from_line + 1
          } else if (op.op === 'update_lines') {
            additions += countLines(op.content)
            deletions += op.to_line - op.from_line + 1
          } else {
            countsKnown = false
          }
        }
        return {
          path: file.path,
          ...resultContext(output, index),
          status: result && !result.success ? 'failed' : 'updated',
          additions: countsKnown ? additions : undefined,
          deletions: countsKnown ? deletions : undefined,
        }
      }),
    }
  }

  if (functionId === DELETE_ID) {
    const req = deleteRequestSchema.safeParse(input)
    if (!req.success) return null
    return {
      action: 'deleted',
      rows: req.data.paths.map((path, index) => {
        const result = resultAt(output, index)
        return {
          path,
          ...resultContext(output, index),
          status: result && !result.success ? 'failed' : result?.removed === false ? 'unchanged' : 'deleted',
        }
      }),
    }
  }

  return null
}
