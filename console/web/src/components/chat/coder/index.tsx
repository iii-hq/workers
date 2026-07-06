import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { ApplyPatchPreview, ApplyPatchView } from './ApplyPatchView'
import { ContextView } from './ContextView'
import { CreateFilePreview, CreateFileView } from './CreateFileView'
import { DeleteFilePreview, DeleteFileView } from './DeleteFileView'
import { InfoView } from './InfoView'
import { ListFolderView } from './ListFolderView'
import { MovePreview, MoveView } from './MoveView'
import {
  isCoderFunction,
  isCoderMutateFunction,
  unwrapEnvelope,
} from './parsers'
import { ReadFileView } from './ReadFileView'
import { SearchView } from './SearchView'
import { TreeView } from './TreeView'
import { UpdateFilePreview, UpdateFileView } from './UpdateFileView'
import { WorktreeAddView, WorktreeRemoveView } from './WorktreeView'

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

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
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
    case 'coder::apply-patch':
      return <ApplyPatchView input={input} output={output} running={running} />
    case 'coder::context':
      return <ContextView input={input} output={output} running={running} />
    case 'coder::worktree-add':
      return <WorktreeAddView input={input} output={output} running={running} />
    case 'coder::worktree-remove':
      return (
        <WorktreeRemoveView input={input} output={output} running={running} />
      )
    default:
      return null
  }
}

/** Only the mutators (create/update/delete/move) gate on approval — the
 *  read-side functions never reach the pending state, so they have no
 *  Preview components to dispatch to. */
function tryRenderPreview(
  message: FunctionCallMessage,
): React.ReactNode | null {
  if (!isCoderMutateFunction(message.functionId)) return null
  const input = unwrapEnvelope(message.input)
  switch (message.functionId) {
    case 'coder::create-file':
      return <CreateFilePreview input={input} />
    case 'coder::update-file':
      return <UpdateFilePreview input={input} />
    case 'coder::delete-file':
      return <DeleteFilePreview input={input} />
    case 'coder::move':
      return <MovePreview input={input} />
    case 'coder::apply-patch':
      return <ApplyPatchPreview input={input} />
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
