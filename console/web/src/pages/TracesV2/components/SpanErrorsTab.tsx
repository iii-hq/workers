import { AlertCircle, CheckCircle2, Copy } from 'lucide-react'
import { useMemo } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
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
    <div className="p-5 space-y-4">
      {/* Error cell with alert stripe */}
      <div className="border border-rule border-l-2 border-l-alert bg-alert/5 p-4">
        <div className="flex items-start gap-3">
          <span className="size-[18px] shrink-0 flex items-center justify-center text-alert">
            <AlertCircle className="w-4 h-4" />
          </span>
          <div className="flex-1 min-w-0">
            {displayType && (
              <div className="font-mono text-[13px] font-semibold text-alert mb-1 lowercase">
                {displayType}
              </div>
            )}
            {displayMessage && (
              <div className="font-mono text-[13px] text-ink break-words leading-relaxed">
                {displayMessage}
              </div>
            )}
            {!displayType && !displayMessage && (
              <div className="font-mono text-[13px] text-ink-faint lowercase">
                error status with no additional details
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Stack trace */}
      {displayStack && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              stack trace
            </span>
            <button
              type="button"
              onClick={copyStackTrace}
              className="flex items-center gap-1.5 px-2 py-1 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint hover:text-ink hover:bg-panel transition-colors"
            >
              {copiedKey === 'stackTrace' ? (
                <span className="text-accent">copied</span>
              ) : (
                <>
                  <Copy className="w-2.5 h-2.5" />
                  <span>copy</span>
                </>
              )}
            </button>
          </div>
          <pre className="border border-rule bg-bg px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink overflow-x-auto whitespace-pre-wrap break-words max-h-[400px] overflow-y-auto">
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
                        ? 'text-ink hover:bg-panel'
                        : 'text-ink-faint'
                      : 'text-alert font-medium'
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
        <div className="border border-rule bg-bg p-4 text-center">
          <p className="font-mono text-[13px] text-ink-faint lowercase">
            no additional error details
          </p>
          <p className="font-mono text-[11px] text-ink-ghost mt-1 lowercase">
            the span is marked as error but no error attributes were recorded
          </p>
        </div>
      )}
    </div>
  )
}
