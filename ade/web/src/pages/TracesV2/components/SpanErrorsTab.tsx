import { AlertCircle, CheckCircle2, Copy } from 'lucide-react'
import { useMemo } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import { cn } from '@/lib/utils'
import { redactValue } from '../lib/redactAttributes'
import type { VisualizationSpan } from '../lib/traceTransform'
import { useCopyToClipboard } from '../lib/traceUtils'

interface SpanErrorsTabProps {
  span: VisualizationSpan
  /**
   * The span's function-trigger redactor (`spanRawRedactor` in
   * functionTriggerFromSpan.ts). `exception.message`/`exception.stacktrace`
   * read off the SAME `exception` event `functionTriggerFromSpan.ts`'s
   * `exceptionOutput()` reads to build the info tab's redacted error output
   * — one tab-click away here otherwise, same as the tags/logs tabs.
   */
  redact?: (value: unknown) => unknown
}

export function SpanErrorsTab({ span, redact }: SpanErrorsTabProps) {
  const { copiedKey, copy } = useCopyToClipboard()
  const exceptionEvent = span.events?.find(
    (e) => e.name === 'exception' || e.name?.startsWith('exception'),
  )
  const hasError = span.status === 'error' || !!exceptionEvent
  const eventAttrs = exceptionEvent?.attributes ?? {}

  const errorMessage = span.attributes?.['error.message'] as string | undefined
  const errorType = span.attributes?.['error.type'] as string | undefined
  const errorStack = span.attributes?.['error.stack'] as string | undefined
  const exceptionMessage = (span.attributes?.['exception.message'] ??
    eventAttrs['exception.message']) as string | undefined
  const exceptionType = (span.attributes?.['exception.type'] ??
    eventAttrs['exception.type']) as string | undefined
  const exceptionStacktrace = (span.attributes?.['exception.stacktrace'] ??
    eventAttrs['exception.stacktrace']) as string | undefined

  // Redacted ONCE here — both the render below and the stack-trace copy
  // button read from these, so they cannot disagree about what is safe.
  const displayMessage = redactValue(
    errorMessage || exceptionMessage,
    redact,
  ) as string | undefined
  const displayType = redactValue(errorType || exceptionType, redact) as
    | string
    | undefined
  const displayStack = redactValue(errorStack || exceptionStacktrace, redact) as
    | string
    | undefined

  // Stack-trace lines are static for a given span and may legitimately
  // repeat (e.g., recursive frames). Stamp each line with a position-
  // derived id once so the render below doesn't key from array index.
  const stackLines = useMemo(
    () =>
      (displayStack ?? '').split('\n').map((line, i) => ({
        line,
        id: `${i}:${line}`,
      })),
    [displayStack],
  )

  const copyStackTrace = () => {
    if (displayStack) {
      copy('stackTrace', displayStack)
    }
  }

  if (!hasError) {
    return (
      <div className="p-4">
        <EmptyState
          icon={CheckCircle2}
          title="no errors"
          description="this span completed successfully"
        />
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4 p-4 font-inter">
      {/* The error itself */}
      <div
        role="alert"
        className="rounded-lg border border-alert/30 bg-alert-muted px-3 py-2.5"
      >
        <div className="flex items-start gap-2.5">
          <AlertCircle
            className="mt-0.5 size-4 shrink-0 text-alert"
            aria-hidden
          />
          <div className="min-w-0 flex-1">
            {displayType && (
              <div className="mb-0.5 font-mono text-xs font-semibold text-alert-strong">
                {displayType}
              </div>
            )}
            {displayMessage && (
              <div className="font-mono text-xs leading-relaxed break-words whitespace-pre-wrap text-foreground">
                {displayMessage}
              </div>
            )}
            {!displayType && !displayMessage && (
              <div className="text-[13px] text-muted-foreground">
                Error status with no additional details
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Stack trace */}
      {displayStack && (
        <div>
          <div className="mb-1.5 flex h-6 items-center justify-between">
            <span className="text-xs font-semibold text-foreground">
              Stack trace
            </span>
            <button
              type="button"
              onClick={copyStackTrace}
              className={cn(
                'flex h-6 items-center gap-1 rounded-md px-1.5 text-[11px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50',
              )}
            >
              {copiedKey === 'stackTrace' ? (
                <span className="text-ok">Copied</span>
              ) : (
                <>
                  <Copy className="size-4" aria-hidden />
                  <span>Copy</span>
                </>
              )}
            </button>
          </div>
          <pre className="max-h-[400px] overflow-auto overscroll-contain rounded-lg border border-border bg-background px-3 py-2 font-mono text-[12px] leading-[1.55] break-words whitespace-pre-wrap text-foreground">
            {stackLines.map(({ line, id }) => {
              const isFrameLine =
                /^\s+at\s/.test(line) ||
                /\.(ts|js|tsx|jsx|py|go|rs|java)[:(\s]/.test(line)
              const hasLineNumber = /:\d+[:\d]*/.test(line)
              return (
                <div
                  key={id}
                  className={
                    isFrameLine
                      ? hasLineNumber
                        ? 'text-foreground'
                        : 'text-muted-foreground'
                      : 'font-medium text-alert-strong'
                  }
                >
                  {line}
                </div>
              )
            })}
          </pre>
        </div>
      )}

      {!displayMessage && !displayType && !displayStack && (
        <div className="rounded-lg border border-border p-4 text-center">
          <p className="text-[13px] text-foreground">
            No additional error details
          </p>
          <p className="mt-1 text-[11px] text-muted-foreground">
            The span is marked as error but no error attributes were recorded
          </p>
        </div>
      )}
    </div>
  )
}
