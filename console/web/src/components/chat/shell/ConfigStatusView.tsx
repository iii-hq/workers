import { StatusPill } from '@/components/chat/sandbox/shared'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import { cn } from '@/lib/utils'
import { safeParseResponse, shellConfigStatusResponseSchema } from './parsers'

interface ShellConfigStatusViewProps {
  output: unknown
  running?: boolean
}

/**
 * `shell::config-status` — the worker's config-reload health. A slab
 * with the last reload outcome, a rejected-reload counter (warn pill
 * when non-zero), and the last rejection error verbatim. The request
 * payload is ignored server-side, so nothing request-derived renders.
 */
export function ShellConfigStatusView({
  output,
  running,
}: ShellConfigStatusViewProps) {
  const resp =
    output != null
      ? safeParseResponse(shellConfigStatusResponseSchema, output)
      : null

  if (!resp) {
    if (!running) return null
    return (
      <div className="border-t border-rule-2 bg-bg px-3 py-3 font-mono text-[12.5px] text-ink-faint italic">
        checking config…
      </div>
    )
  }

  const applied = resp.last_outcome === 'applied'
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div
        className={cn(
          'px-3 py-3 flex flex-col gap-2 border-l-2',
          applied ? 'border-accent' : 'border-alert',
        )}
      >
        <div className="flex flex-wrap items-center gap-1.5">
          <StatusPill
            label={resp.last_outcome}
            variant={applied ? 'accent' : 'alert'}
          />
          {resp.rejected_reloads > 0 ? (
            <FooterPill tone="warn">
              {`rejected reloads ${resp.rejected_reloads}`}
            </FooterPill>
          ) : (
            <Chip label="rejected reloads">{resp.rejected_reloads}</Chip>
          )}
        </div>
        {resp.last_error != null ? (
          <pre className="font-mono text-[12.5px] leading-[1.55] text-warn whitespace-pre-wrap break-words m-0">
            <code>{resp.last_error}</code>
          </pre>
        ) : null}
      </div>
    </div>
  )
}
