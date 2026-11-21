/**
 * The `<workflow_input>` block a workflow node's session opens with
 * (`build_opening` in workflow/src/functions/tick.rs): the untrusted output
 * of upstream nodes or the run's input, as one line of JSON with `<`/`>`
 * escaped. The console cuts the block out of the user message and hands it
 * here (`host.chat.registerMessageBlockRenderer`); it shows as readable rows
 * with the JSON one disclosure away. It is not anyone's words, so it is a
 * card, not a bubble.
 */

import {
  Card,
  CardBody,
  CardHeader,
  Chip,
  type Host,
  JsonHighlight,
  type MessageBlockProps,
  Tooltip,
} from '@iii-dev/console-ui'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import { Workflow } from 'lucide-react'
import { Fragment, memo, useMemo } from 'react'

/** Feature-detected: a console without the message-block slot shows the raw tag. */
export function registerWorkflowInputBlock(host: Host): (() => void) | undefined {
  return host.chat?.registerMessageBlockRenderer?.({ tag: 'workflow_input', render: WorkflowInputBlock })
}

/** `loose_ends` / `fanoutItem` -> "Loose ends" / "Fanout item". */
function humanizeKey(key: string): string {
  const words = key
    .replace(/([a-z0-9])([A-Z])(?=[a-z])/g, (_, a, b) => `${a} ${b.toLowerCase()}`)
    .replace(/[_-]+/g, ' ')
    .trim()
  return words ? words[0].toUpperCase() + words.slice(1) : JSON.stringify(key)
}

function Quiet({ children }: { children: string }) {
  return <span className="wf-input-quiet">{children}</span>
}

/** An object or a non-empty array: laid out under its label, not beside it. */
function isBlock(value: unknown): boolean {
  return value !== null && typeof value === 'object' && Object.keys(value).length > 0
}

/**
 * The value as a reader would want it. Strings are PLAIN text, never
 * Markdown: this is untrusted upstream output, and Markdown would fetch its
 * remote images and render its links.
 */
function Value({ value }: { value: unknown }) {
  if (typeof value === 'string') {
    return value ? <span className="wf-input-text">{value}</span> : <Quiet>empty</Quiet>
  }
  if (value === null || typeof value !== 'object') {
    return <span className="wf-input-mono">{String(value)}</span>
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <Quiet>none</Quiet>
    const List = value.some(isBlock) ? 'ol' : 'ul'
    return (
      <List className="wf-input-list">
        {value.map((item, i) => (
          // Items have no identity beyond their position.
          <li key={i}>
            <Value value={item} />
          </li>
        ))}
      </List>
    )
  }
  const entries = Object.entries(value)
  if (entries.length === 0) return <Quiet>empty</Quiet>
  return (
    <dl className="wf-input-facts">
      {entries.map(([key, item]) => {
        const block = isBlock(item) ? '' : undefined
        return (
          <Fragment key={key}>
            <dt data-block={block}>{humanizeKey(key)}</dt>
            <dd data-block={block}>
              <Value value={item} />
            </dd>
          </Fragment>
        )
      })}
    </dl>
  )
}

/** The body parsed (which restores the escaped `<`/`>`), or null when it is not JSON. */
function parseBody(body: string): { value: unknown; json: string } | null {
  try {
    const value: unknown = JSON.parse(body)
    return { value, json: JSON.stringify(value, null, 2) }
  } catch {
    return null
  }
}

// Memoized: the transcript re-renders per streamed token, and the props keep their identity.
export const WorkflowInputBlock = memo(function WorkflowInputBlock({ attributes, body }: MessageBlockProps) {
  const note = attributes.note || undefined
  const parsed = useMemo(() => parseBody(body), [body])
  const chip = (
    // A tab stop so a keyboard can open the note's tooltip too.
    <Chip tone="warning" tabIndex={note ? 0 : undefined} className="wf-input-chip">
      Untrusted
      {note ? <span className="wf-sr-only">: {note}</span> : null}
    </Chip>
  )
  return (
    <article className="wf-input" data-message-role="workflow-input" aria-label="Workflow input">
      <Card>
        <CardHeader className="wf-input-head">
          <Workflow aria-hidden className={`${uiClasses.icon} wf-accent`} />
          <span className="wf-input-title">Workflow input</span>
          {note ? <Tooltip label={note}>{chip}</Tooltip> : chip}
        </CardHeader>
        <CardBody className="wf-input-body">
          {/* A long input scrolls inside the card instead of flooding the
              transcript; the tab stop lets a keyboard scroll it. */}
          <section
            aria-label="Workflow input data"
            // biome-ignore lint/a11y/noNoninteractiveTabindex: the scrolling region must be reachable by keyboard
            tabIndex={0}
            className="wf-input-data"
          >
            {parsed ? <Value value={parsed.value} /> : <pre className="wf-input-raw">{body}</pre>}
          </section>
          {parsed ? (
            <details>
              <summary className="wf-input-summary">Show JSON</summary>
              <section
                aria-label="Workflow input JSON"
                // biome-ignore lint/a11y/noNoninteractiveTabindex: the scrolling region must be reachable by keyboard
                tabIndex={0}
                className="wf-input-json"
              >
                <JsonHighlight code={parsed.json} wrap />
              </section>
            </details>
          ) : null}
        </CardBody>
      </Card>
    </article>
  )
})
