import { Workflow } from 'lucide-react'
import { Fragment, memo } from 'react'
import { Chip } from '@/components/ui/Chip'
import {
  FactList,
  factLabelClassName,
  factValueClassName,
} from '@/components/ui/FactCard'
import { Hint } from '@/components/ui/Hint'
import { Card, CardBody, CardHeader } from '@/components/ui/Surface'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { WorkflowInput } from '@/lib/workflow-input'
import { CopyMessageButton } from './CopyMessageButton'

/** `loose_ends` / `fanoutItem` -> "Loose ends" / "Fanout item". */
function humanizeKey(key: string): string {
  const words = key
    .replace(
      /([a-z0-9])([A-Z])(?=[a-z])/g,
      (_, a, b) => `${a} ${b.toLowerCase()}`,
    )
    .replace(/[_-]+/g, ' ')
    .trim()
  return words ? words[0].toUpperCase() + words.slice(1) : JSON.stringify(key)
}

function Quiet({ children }: { children: string }) {
  return <span className="text-ink-faint italic">{children}</span>
}

/** An object or a non-empty array: laid out under its label, not beside it. */
function isBlock(value: unknown): boolean {
  return (
    value !== null &&
    typeof value === 'object' &&
    Object.keys(value as object).length > 0
  )
}

/**
 * The value as a reader would want it. Strings are PLAIN text, never
 * Markdown: this is untrusted upstream output, and Markdown would fetch its
 * remote images and render its links.
 */
function Value({ value }: { value: unknown }) {
  if (typeof value === 'string') {
    return value ? (
      <span
        className={cn(factValueClassName, 'whitespace-pre-wrap break-words')}
      >
        {value}
      </span>
    ) : (
      <Quiet>empty</Quiet>
    )
  }
  if (value === null || typeof value !== 'object') {
    // Geist Mono: `.chat-message` remaps plain `font-mono` to Inter.
    return (
      <span className="font-geist-mono text-body text-ink tabular-nums">
        {String(value)}
      </span>
    )
  }
  if (Array.isArray(value)) {
    if (value.length === 0) return <Quiet>none</Quiet>
    const flat = !value.some(isBlock)
    const List = flat ? 'ul' : 'ol'
    return (
      <List
        className={cn(
          'm-0 pl-5 marker:text-ink-faint',
          flat ? 'list-disc space-y-1' : 'list-decimal space-y-3',
        )}
      >
        {value.map((item, i) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: items have no identity beyond their position
          <li key={i} className="min-w-0 break-words">
            <Value value={item} />
          </li>
        ))}
      </List>
    )
  }
  const entries = Object.entries(value)
  if (entries.length === 0) return <Quiet>empty</Quiet>
  return (
    <FactList className="min-w-0 gap-y-2">
      {entries.map(([key, item]) => {
        const block = isBlock(item)
        return (
          <Fragment key={key}>
            <dt
              className={cn(
                factLabelClassName,
                'min-w-0 wrap-anywhere',
                block && 'col-span-2',
              )}
            >
              {humanizeKey(key)}
            </dt>
            <dd
              className={cn(
                'm-0 min-w-0 break-words',
                block && 'col-span-2 border-l border-edge pl-3',
              )}
            >
              <Value value={item} />
            </dd>
          </Fragment>
        )
      })}
    </FactList>
  )
}

/**
 * The data a workflow node was handed (`<workflow_input>`): untrusted output
 * of upstream nodes or the run's input, shown as readable rows with the JSON
 * one disclosure away. It is not anyone's words, so it is a card, not a bubble.
 */
export const WorkflowInputCard = memo(function WorkflowInputCard({
  input,
  copyText,
}: {
  input: WorkflowInput
  /** The original message, when no bubble above carries its copy action. */
  copyText?: string
}) {
  return (
    <article
      className="w-full min-w-0"
      data-message-role="workflow-input"
      aria-label="Workflow input"
    >
      <Card>
        <CardHeader className="border-b border-edge">
          <Workflow aria-hidden className="size-4 shrink-0 stroke-accent" />
          <span className="min-w-0 flex-1 truncate font-geist-mono text-[0.8125rem] font-medium tracking-[0.06em]">
            Workflow input
          </span>
          <Hint label={input.note} side="top" disabled={!input.note}>
            <Chip
              tone="warning"
              // A tab stop so a keyboard can open the note's tooltip too.
              tabIndex={input.note ? 0 : undefined}
              className="shrink-0 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
            >
              Untrusted
              {input.note ? (
                <span className="sr-only">: {input.note}</span>
              ) : null}
            </Chip>
          </Hint>
          {copyText ? (
            <Hint label="Copy message" side="top">
              <CopyMessageButton text={copyText} />
            </Hint>
          ) : null}
        </CardHeader>
        <CardBody className="flex flex-col gap-3">
          {/* A long input scrolls inside the card instead of flooding the
              transcript; the tab stop lets a keyboard scroll it. */}
          <section
            aria-label="Workflow input data"
            // biome-ignore lint/a11y/noNoninteractiveTabindex: the scrolling region must be reachable by keyboard
            tabIndex={0}
            className="max-h-96 min-w-0 overflow-y-auto focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:ring-inset focus-visible:outline-none"
          >
            {input.parsed ? (
              <Value value={input.value} />
            ) : (
              <pre className="m-0 font-code text-[12.5px] leading-[1.55] whitespace-pre-wrap break-words text-ink">
                {input.raw}
              </pre>
            )}
          </section>
          {input.parsed ? (
            <details>
              {/* A card's section label: 12px / 500 ink-faint (design-system.md). */}
              <summary className="cursor-pointer list-none select-none text-xs font-medium text-ink-faint transition-colors hover:text-ink focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none">
                Show JSON
              </summary>
              <div className="mt-1 max-h-64 overflow-auto border border-border">
                <JsonHighlight
                  code={JSON.stringify(input.value, null, 2)}
                  wrap
                />
              </div>
            </details>
          ) : null}
        </CardBody>
      </Card>
    </article>
  )
})
