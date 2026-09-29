/**
 * Injected function-trigger renderer for every `computer::*` call —
 * registered through `host.functionTriggers`, so it dispatches before the
 * console's built-in families and owns how computer calls render in chat and
 * in the traces span tab. Screenshots render inline (that image IS the
 * result), actions collapse to one line, and anything unrecognised falls back
 * to the decoded JSON. Errors are left to the console's own cards.
 */

import {
  type FunctionTriggerMessage,
  type FunctionTriggerRenderer,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import {
  decodeComputerResult,
  isComputerFunction,
  sessionIdFromCall,
} from '../lib/computer'
import {
  ActView,
  CaptureView,
  DisplaysView,
  SessionListView,
  SessionStartView,
  SessionStopView,
} from './ComputerViews'

/** The injected page's route — where "open the desktop" navigates. */

/**
 * Header label for `computer::*` ids: dims the namespace prefix so the op
 * (`act`, `screenshot`, …) reads clearly.
 */
function FunctionIdLabel({ functionId }: { functionId: string }) {
  if (!functionId.startsWith('computer::')) {
    return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  }
  const tail = functionId.slice('computer::'.length)
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>computer::</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{tail}</span>
    </>
  )
}

function renderBody(message: FunctionTriggerMessage): React.ReactNode | null {
  const { input, output } = message
  switch (message.functionId) {
    case 'computer::sessions::start':
      return <SessionStartView output={output} />
    case 'computer::sessions::list':
      return <SessionListView output={output} />
    case 'computer::sessions::stop':
      return <SessionStopView output={output} />
    case 'computer::displays':
      return <DisplaysView output={output} />
    case 'computer::screenshot':
    case 'computer::observe':
      return <CaptureView output={output} />
    case 'computer::act':
      return <ActView input={input} output={output} />
    default:
      return null
  }
}

function ComputerCallView({
  host,
  message,
}: {
  host: Host
  message: FunctionTriggerMessage
}) {
  const sessionId = sessionIdFromCall(message.input, message.output)
  const running = !!message.running

  const body = !running && message.output != null ? renderBody(message) : null
  const fallback =
    !body && message.output != null
      ? decodeComputerResult(message.output)
      : null

  return (
    <div className="cp-ui-call">
      <div className="cp-ui-call-head">
        <span className="cp-ui-call-session">
          {sessionId ? (
            <>
              session <span className="cp-ui-call-sid">{sessionId}</span>
            </>
          ) : (
            'computer'
          )}
        </span>
        {sessionId ? (
          <button
            type="button"
            className="cp-ui-call-link"
            onClick={() => host.panels?.open({ pageId: 'computer' })}
          >
            open the desktop
          </button>
        ) : null}
      </div>
      {running && message.output == null ? (
        <p className="cp-ui-line">running...</p>
      ) : body ? (
        body
      ) : fallback != null ? (
        <div className="cp-ui-json">
          <JsonHighlight code={JSON.stringify(fallback, null, 2)} />
        </div>
      ) : (
        <p className="cp-ui-line">no result</p>
      )}
    </div>
  )
}

function renderCall(
  host: Host,
  message: FunctionTriggerMessage,
): React.ReactNode | null {
  if (!isComputerFunction(message.functionId)) return null
  if (message.pendingApproval) return null
  return <ComputerCallView host={host} message={message} />
}

export function createComputerRenderer(host: Host): FunctionTriggerRenderer {
  return {
    id: 'computer/page.js#calls',
    isMatch: isComputerFunction,
    tryRender: (message) => renderCall(host, message),
    tryRenderRunning: (message) => renderCall(host, message),
    tryRenderPreview: () => null,
    FunctionIdLabel,
  }
}
