import type { FunctionTriggerMessage, FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import { ErrorDisplayView } from '../lib/errors'
import { FileChangesCard } from './FileChangesCard'
import { diffPanelRequest, isFileChangesResponse, summarizeFileChanges } from './file-changes'
import { parseShellErrorDisplay, unwrapEnvelope } from './parsers'

const CREATE_ID = 'coder::create-file'
const UPDATE_ID = 'coder::update-file'
const DELETE_ID = 'coder::delete-file'
const FILE_CHANGE_IDS = new Set([CREATE_ID, UPDATE_ID, DELETE_ID])

function render(host: Host, message: FunctionTriggerMessage): React.ReactNode | null {
  if (!FILE_CHANGE_IDS.has(message.functionId)) return null
  const rawOutput = message.output
  const input = unwrapEnvelope(message.input)
  const output = rawOutput == null ? undefined : unwrapEnvelope(rawOutput)

  // While running, the request is enough for a useful preview. Once settled,
  // require the coder response shape before declaring any file successful;
  // gate/transport errors must fall through to the shared error renderer.
  const summary =
    message.running || isFileChangesResponse(output) ? summarizeFileChanges(message.functionId, input, output) : null
  if (summary) {
    return (
      <FileChangesCard
        summary={summary}
        running={!!message.running}
        onOpenDiff={host.panels ? (row) => host.panels?.open(diffPanelRequest(row)) : undefined}
      />
    )
  }

  const error = !message.running && rawOutput != null ? parseShellErrorDisplay(rawOutput) : null
  if (error) return <ErrorDisplayView display={error} />
  return null
}

function FunctionIdLabel({ functionId }: { functionId: string }) {
  const tail = functionId.startsWith('coder::') ? functionId.slice('coder::'.length) : functionId
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>coder::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{tail}</span>
    </>
  )
}

export function createFileChangesRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'shell/page.js#file-changes',
    isMatch: (functionId) => FILE_CHANGE_IDS.has(functionId),
    tryRender: (message) => render(host, message),
    tryRenderRunning: (message) => render(host, message),
    tryRenderPreview: (message) => render(host, message),
    FunctionIdLabel,
    metadata: { display: true },
  }
}
