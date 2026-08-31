import { ChevronRight, CircleAlert, ExternalLink, Wrench } from 'lucide-react'
import { type ReactNode, useId } from 'react'
import { Chip } from '@/components/ui/Chip'
import {
  Card,
  CardBody,
  CardHeader,
  CardHighlight,
} from '@/components/ui/Surface'
import { cn } from '@/lib/utils'

export interface ErrorCardMetadata {
  label: string
  value: ReactNode
}

export interface ErrorCardProps {
  badge: string
  title: string
  message: string
  category?: string
  retryable?: boolean
  metadata?: readonly ErrorCardMetadata[]
  guidanceTitle?: string
  guidance?: ReactNode
  technicalDetails?: string
  docsUrl?: string
  output?: ReactNode
  className?: string
}

/** Shared, user-facing error surface for tool and invocation failures. */
export function ErrorCard({
  badge,
  title,
  message,
  category,
  retryable,
  metadata = [],
  guidanceTitle = 'Suggested fix',
  guidance,
  technicalDetails,
  docsUrl,
  output,
  className,
}: ErrorCardProps) {
  const titleId = useId()

  return (
    <Card
      className={cn('@container', className)}
      data-error-card=""
      data-error-retryable={retryable || undefined}
      role="alert"
      aria-labelledby={titleId}
    >
      <CardHeader className="items-start border-b border-edge p-4 sm:p-3">
        <CircleAlert
          aria-hidden
          className="size-5 shrink-0 stroke-warn sm:size-4"
        />
        <div className="flex min-w-0 flex-1 flex-col gap-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <div
              id={titleId}
              className="min-w-0 text-balance font-sans text-base font-semibold text-ink sm:text-sm"
            >
              {title}
            </div>
            {category ? (
              <div className="font-mono text-[0.6875rem] font-medium uppercase tracking-wide text-ink-faint/40">
                {category}
              </div>
            ) : null}
            <div className="flex-1" />
            <Chip tone="warning">{badge}</Chip>
            {retryable ? <Chip tone="accent">Retryable</Chip> : null}
          </div>

        </div>
      </CardHeader>

      <CardBody className="p-0">
        <div className="flex min-w-0 flex-col gap-4">
          <p className="whitespace-pre-wrap break-words text-pretty font-sans text-base text-ink sm:text-sm">
            {message}
          </p>

          {metadata.length > 0 ? (
            <dl className="flex min-w-0 flex-col gap-3">
              {metadata.map((item) => (
                <div
                  key={item.label}
                  className="flex min-w-0 flex-col gap-1 @md:flex-row @md:gap-4"
                >
                  <dt className="shrink-0 font-mono text-[0.6875rem] font-medium uppercase tracking-wide text-ink @md:w-28">
                    {item.label}
                  </dt>
                  <dd className="min-w-0 break-words font-mono text-base text-ink-faint sm:text-[0.8125rem]">
                    {item.value}
                  </dd>
                </div>
              ))}
            </dl>
          ) : null}
        </div>

        {guidance ? (
          <div className="border-t border-edge p-4 sm:p-3">
            <CardHighlight className="flex items-start gap-3 p-3">
              <Wrench
                aria-hidden
                className="size-5 shrink-0 stroke-warn sm:size-4"
              />
              <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                <div className="font-sans text-base font-semibold text-ink sm:text-sm">
                  {guidanceTitle}
                </div>
                <div className="text-pretty font-sans text-base text-ink-faint sm:text-sm">
                  {guidance}
                </div>
              </div>
            </CardHighlight>
          </div>
        ) : null}

        {output ? (
          <section
            className="border-t border-edge"
            aria-label="Partial command output"
          >
            <div className="bg-surface px-4 py-2 font-mono text-[0.6875rem] font-medium uppercase tracking-wide text-ink-faint sm:px-3">
              Partial output
            </div>
            {output}
          </section>
        ) : null}

        {technicalDetails ? (
          <details className="group border-t border-edge">
            <summary className="relative flex min-w-0 cursor-pointer list-none items-center gap-2 p-4 font-sans text-base font-medium text-ink-faint select-none hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-accent sm:p-3 sm:text-sm">
              <span
                className="pointer-events-none absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                aria-hidden="true"
              />
              <ChevronRight
                aria-hidden
                className="size-5 shrink-0 stroke-ink-faint transition-transform duration-150 group-open:rotate-90 motion-reduce:transition-none sm:size-4"
              />
              <span>Technical details</span>
            </summary>
            <pre className="max-h-64 overflow-auto border-t border-edge bg-bg p-4 font-mono text-base whitespace-pre-wrap break-words text-ink-faint sm:p-3 sm:text-[0.8125rem]">
              <code>{technicalDetails}</code>
            </pre>
          </details>
        ) : null}

        {docsUrl ? (
          <div className="border-t border-edge p-4 font-sans text-base font-medium sm:p-3 sm:text-sm">
            <a
              href={docsUrl}
              target="_blank"
              rel="noreferrer noopener"
              className="flex w-fit items-center gap-2 text-accent hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
            >
              <span>Open documentation</span>
              <ExternalLink
                aria-hidden
                className="size-5 shrink-0 stroke-accent sm:size-4"
              />
            </a>
          </div>
        ) : null}
      </CardBody>
    </Card>
  )
}
