import { JsonHighlight, StatusDot, StatusPanel } from '@iii-dev/console-ui'
import { memo, useEffect, useState } from 'react'
import {
  type Block,
  blocksOf,
  drawsWhole,
  elapsedText,
  highlightable,
  jsonText,
  messagesOf,
  resultSummary,
} from './presentation'

/** Highlighted when `json`, plain text otherwise. */
function Code({ code, json }: { code: string; json: boolean }) {
  return json ? <JsonHighlight code={code} className="mcp-ui-json" wrap /> : <pre className="mcp-ui-code">{code}</pre>
}

/** Each block in order: text that parses as JSON highlighted, other text as code, an image as an image. */
function Blocks({ blocks }: { blocks: Block[] }) {
  return blocks.map((block, i) => {
    const key = `${i}:${block.type}`
    if (block.type === 'image') {
      return (
        <img
          key={key}
          className="mcp-ui-image"
          // biome-ignore lint/a11y/noRedundantAlt: the spec's copy; nothing else names what the server sent
          alt="Image result"
          src={`data:${block.mime};base64,${block.data}`}
        />
      )
    }
    const code = jsonText(block.text)
    return <Code key={key} code={code ?? block.text} json={code !== null} />
  })
}

function Raw({ result }: { result: unknown }) {
  const code = JSON.stringify(result, null, 2) ?? ''
  return <Code code={code} json={highlightable(code)} />
}

/** Seconds since the call started, counted only while it runs. */
function useSeconds(running: boolean): number {
  const [seconds, setSeconds] = useState(0)
  useEffect(() => {
    if (!running) return
    setSeconds(0)
    const timer = setInterval(() => setSeconds((n) => n + 1), 1000)
    return () => clearInterval(timer)
  }, [running])
  return seconds
}

/** The Result half of the Test tab: a status line, each block (or a prompt's messages) drawn, the raw
 * result behind a disclosure.
 * Memoised: the panel re-renders on every keystroke and live read, and a long highlighted result
 * would redraw with each one. */
export const ResultView = memo(function ResultView({
  result,
  error,
  ms,
  running,
}: {
  result: unknown | null
  error: string | null
  ms: number | null
  running: boolean
}) {
  const seconds = useSeconds(running)
  const ok = !running && ms !== null && error === null
  // A prompt's messages; a tool or resource result has none and no description.
  const prompt = messagesOf(result)
  return (
    <div className="mcp-ui-result">
      {/* The seconds stay out of the announcement: it says Running once, then the outcome. */}
      <p className="mcp-ui-result-status" role="status">
        {running ? (
          <>
            <StatusDot tone="accent" pulse />
            Running<span aria-hidden="true"> · {seconds} s</span>
          </>
        ) : ms === null ? (
          'Not run yet'
        ) : error !== null ? (
          <>
            <StatusDot tone="alert" />
            Error · {elapsedText(ms)}
          </>
        ) : (
          <>
            <StatusDot tone="ok" />
            {resultSummary(result, ms)}
          </>
        )}
      </p>
      {!running && error !== null ? <StatusPanel variant="alert" headline="The call failed" detail={error} /> : null}
      {ok && drawsWhole(result) ? <Raw result={result} /> : null}
      {ok && !drawsWhole(result) ? (
        <>
          {prompt.description ? <p className="mcp-ui-quiet-line">{prompt.description}</p> : null}
          {prompt.messages.map((message, i) => (
            <div key={i} className="mcp-ui-message">
              <h3 className="mcp-ui-role">{message.role}</h3>
              <Blocks blocks={message.blocks} />
            </div>
          ))}
          <Blocks blocks={blocksOf(result)} />
          <details className="mcp-ui-raw">
            <summary>Raw result</summary>
            <Raw result={result} />
          </details>
        </>
      ) : null}
    </div>
  )
})
