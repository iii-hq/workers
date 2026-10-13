import type { FunctionTriggerMessage, FunctionTriggerRenderer, Host } from '@iii-dev/console-ui'
import { ErrorDisplayView } from '../lib/errors'
import { FindRelevantCard } from './FindRelevantCard'
import { FIND_RELEVANT_ID, isFindRelevantResponse, openFileRequest, summarizeFindRelevant } from './find-relevant'
import { parseShellErrorDisplay, unwrapEnvelope } from './parsers'
import { FunctionIdLabel } from './shared'

function render(host: Host, message: FunctionTriggerMessage): React.ReactNode | null {
  if (message.functionId !== FIND_RELEVANT_ID) return null
  const rawOutput = message.output
  const input = unwrapEnvelope(message.input)
  const output = rawOutput == null ? undefined : unwrapEnvelope(rawOutput)

  // In flight, the question is the card. Settled, only the find-relevant
  // response shape renders the ranking; C210 input errors, gate denials and
  // transport failures fall through to the shared error card.
  const settled = !message.running && output !== undefined
  if (!settled || isFindRelevantResponse(output)) {
    const summary = summarizeFindRelevant(input, settled ? output : undefined)
    if (summary) {
      const open = host.panels
        ? (path: string, lineFrom?: number, lineTo?: number) =>
            host.panels?.open(openFileRequest(path, lineFrom, lineTo))
        : undefined
      return <FindRelevantCard summary={summary} running={!!message.running} onOpen={open} />
    }
  }

  const error = settled && rawOutput != null ? parseShellErrorDisplay(rawOutput) : null
  if (error) return <ErrorDisplayView display={error} />
  return null
}

export function createFindRelevantRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'ide/page.js#find-relevant',
    isMatch: (functionId) => functionId === FIND_RELEVANT_ID,
    tryRender: (message) => render(host, message),
    tryRenderRunning: (message) => render(host, message),
    tryRenderPreview: (message) => render(host, message),
    FunctionIdLabel,
    metadata: { display: true },
  }
}
