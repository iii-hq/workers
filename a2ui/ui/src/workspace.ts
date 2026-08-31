import type { Host } from '@iii-dev/console-ui'
import type { CodeExport } from './types'
import { unwrapEnvelope } from './types'

interface CreateFileResult {
  path: string
  success: boolean
  error?: { code?: string; message?: string }
}

interface CreateFileResponse {
  results: CreateFileResult[]
}

export interface WorkspaceExport {
  directory: string
  files: string[]
  written: number
  existing: number
}

export async function writeCodeBundle(
  host: Host,
  workingDir: string,
  bundle: CodeExport,
  revision: number,
): Promise<WorkspaceExport> {
  const directory = projectDirectory(workingDir, bundle.surface_id, revision)
  const files = bundle.files.map((file) => ({
    path: joinPath(directory, assertSafeRelativePath(file.path)),
    content: file.content,
    parents: true,
    overwrite: false,
  }))
  const response = unwrapEnvelope(
    await host.iii.trigger('coder::create-file', { files }),
  ) as CreateFileResponse
  if (!Array.isArray(response?.results) || response.results.length !== files.length) {
    throw new Error('Shell did not return a result for every generated file')
  }
  const failures = response.results.filter(
    (result) => !result.success && result.error?.code !== 'C213',
  )
  if (failures.length > 0) {
    throw new Error(
      failures
        .map((result) => `${result.path}: ${result.error?.message ?? 'write failed'}`)
        .join('\n'),
    )
  }
  return {
    directory,
    files: response.results.map((result) => result.path),
    written: response.results.filter((result) => result.success).length,
    existing: response.results.filter((result) => result.error?.code === 'C213').length,
  }
}

export function projectDirectory(workingDir: string, surfaceId: string, revision: number): string {
  const root = workingDir.trim()
  if (!root || root.includes('\0')) throw new Error('Choose a valid working directory in Harness first')
  return joinPath(root, `generated/a2ui/${safeSegment(surfaceId)}-r${revision}`)
}

function assertSafeRelativePath(path: string): string {
  const parts = path.split('/')
  if (
    !path ||
    path.includes('\\') ||
    path.includes('\0') ||
    path.startsWith('/') ||
    /^[a-z]:/i.test(path) ||
    parts.some((part) => !part || part === '.' || part === '..')
  ) {
    throw new Error(`Generated bundle contains an unsafe path: ${path}`)
  }
  return path
}

function safeSegment(value: string): string {
  return value.replace(/[^a-z0-9._-]+/gi, '-').replace(/^-+|-+$/g, '') || 'surface'
}

function joinPath(root: string, relative: string): string {
  const separator = root.includes('\\') && !root.includes('/') ? '\\' : '/'
  const trimmedRoot = root.replace(/[\\/]+$/, '')
  const joinedRelative = relative.split('/').join(separator)
  return trimmedRoot ? `${trimmedRoot}${separator}${joinedRelative}` : `${separator}${joinedRelative}`
}
