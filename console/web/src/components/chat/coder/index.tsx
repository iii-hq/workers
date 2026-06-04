import { SandboxErrorView } from '@/components/chat/sandbox/ErrorView'
import { parseSandboxErrorDisplay } from '@/components/chat/sandbox/parsers'
import type { FunctionCallMessage } from '@/types/chat'
import { CreateFilePreview, CreateFileView } from './CreateFileView'
import { DeleteFilePreview, DeleteFileView } from './DeleteFileView'
import { isCoderMutateFunction, unwrapEnvelope } from './parsers'
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

function tryRender(message: FunctionCallMessage): React.ReactNode | null {
  if (!isCoderMutateFunction(message.functionId)) return null
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
    default:
      return null
  }
}

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
    default:
      return null
  }
}

export const CoderToolView = {
  isCoderMutateFunction,
  tryRender,
  tryRenderRunning: tryRender,
  tryRenderPreview,
}
