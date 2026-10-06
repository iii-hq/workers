import type { z } from 'zod'
import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionTriggerMessage } from '@/types/chat'
import { CreateFilePreview, CreateFileView } from './CreateFileView'
import { DeleteFilePreview, DeleteFileView } from './DeleteFileView'
import { InfoView } from './InfoView'
import { ListFolderView } from './ListFolderView'
import { MovePreview, MoveView } from './MoveView'
import {
  createFileRequestSchema,
  deleteFileRequestSchema,
  isCoderFunction,
  isCoderMutateFunction,
  moveFileRequestSchema,
  safeParseRequest,
  unwrapEnvelope,
  updateFileRequestSchema,
} from './parsers'
import { ReadFileView } from './ReadFileView'
import { SearchView } from './SearchView'
import { TreeView } from './TreeView'
import { UpdateFilePreview, UpdateFileView } from './UpdateFileView'

export function CoderFunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('coder::')) {
    return <span className="text-ink">{functionId}</span>
  }
  const tail = functionId.slice('coder::'.length)
  return (
    <>
      <span className="text-ink-faint">coder::</span>
      <span className="text-ink font-medium">{tail}</span>
    </>
  )
}

function tryRender(message: FunctionTriggerMessage): React.ReactNode | null {
  if (!isCoderFunction(message.functionId)) return null
  if (message.pendingApproval) return null

  const input = unwrapEnvelope(message.input)
  const rawOutput = message.output
  const output = rawOutput != null ? unwrapEnvelope(rawOutput) : undefined
  const running = !!message.running

  const errorDisplay =
    !running && rawOutput != null ? parseSandboxErrorDisplay(rawOutput) : null
  if (errorDisplay) {
    return <SandboxErrorView display={errorDisplay} />
  }

  switch (message.functionId) {
    case 'coder::create-file':
      return <CreateFileView input={input} output={output} running={running} />
    case 'coder::update-file':
      return <UpdateFileView input={input} output={output} running={running} />
    case 'coder::delete-file':
      return <DeleteFileView input={input} output={output} running={running} />
    case 'coder::move':
      return <MoveView input={input} output={output} running={running} />
    case 'coder::read-file':
      return <ReadFileView input={input} output={output} running={running} />
    case 'coder::search':
      return <SearchView input={input} output={output} running={running} />
    case 'coder::tree':
      return <TreeView input={input} output={output} running={running} />
    case 'coder::list-folder':
      return <ListFolderView input={input} output={output} running={running} />
    case 'coder::info':
      return <InfoView input={input} output={output} running={running} />
    default:
      return null
  }
}

const PREVIEW_REQUEST_SCHEMAS: Record<string, z.ZodType> = {
  'coder::create-file': createFileRequestSchema,
  'coder::update-file': updateFileRequestSchema,
  'coder::delete-file': deleteFileRequestSchema,
  'coder::move': moveFileRequestSchema,
}

/** Only the mutators (create/update/delete/move) gate on approval — the
 *  read-side functions never reach the pending state, so they have no
 *  Preview components to dispatch to. A request the preview cannot parse
 *  returns null so the card shows the raw request instead of an empty
 *  preview above Approve. */
function tryRenderPreview(
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  const schema = PREVIEW_REQUEST_SCHEMAS[message.functionId]
  if (!schema) return null
  const input = unwrapEnvelope(message.input)
  if (!safeParseRequest(schema, input)) return null
  switch (message.functionId) {
    case 'coder::create-file':
      return <CreateFilePreview input={input} />
    case 'coder::update-file':
      return <UpdateFilePreview input={input} />
    case 'coder::delete-file':
      return <DeleteFilePreview input={input} />
    case 'coder::move':
      return <MovePreview input={input} />
    default:
      return null
  }
}

export const CoderToolView = {
  isCoderFunction,
  isCoderMutateFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
