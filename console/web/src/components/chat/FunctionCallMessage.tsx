import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { StatusDot } from '@/components/ui/StatusDot'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import type { FunctionCallMessage as FunctionCallMessageType } from '@/types/chat'

interface FunctionCallMessageProps {
  message: FunctionCallMessageType
  defaultOpen?: boolean
  onApprove?: () => void
  onDeny?: () => void
}

function formatJson(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}

type Primitive = string | number | boolean | null

function isPrimitive(v: unknown): v is Primitive {
  return (
    v === null ||
    typeof v === 'string' ||
    typeof v === 'number' ||
    typeof v === 'boolean'
  )
}

/**
 * `null`, `undefined`, `""`, `[]`, and `{}` count as empty so we can render a
 * compact "· empty" header instead of a noisy `{}` JSON block. `null` is
 * intentionally treated as empty (the user-facing concept is "no input"); the
 * primitive `null` rendering would otherwise show the literal text "null".
 */
function isEmptyValue(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function singlePrimitiveField(
  v: unknown,
): { key: string; value: Primitive } | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null
  const entries = Object.entries(v as Record<string, unknown>)
  if (entries.length !== 1) return null
  const [key, value] = entries[0]
  if (!isPrimitive(value)) return null
  return { key, value }
}

function formatPrimitive(v: Primitive): string {
  if (v === null) return 'null'
  return String(v)
}

export function FunctionCallMessage({
  message,
  defaultOpen,
  onApprove,
  onDeny,
}: FunctionCallMessageProps) {
  const pending = !!message.pendingApproval
  const running = !!message.running
  const [open, setOpen] = useState(!!defaultOpen || pending)

  useEffect(() => {
    if (pending) setOpen(true)
  }, [pending])

  const dotTone: 'accent' | 'warn' | 'ink' = pending
    ? 'warn'
    : running
      ? 'accent'
      : 'ink'

  return (
    <div className="border border-rule bg-bg">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          'w-full flex items-center justify-between gap-3 px-3 py-2 cursor-pointer text-left',
          'hover:bg-paper-2 transition-colors',
        )}
      >
        <span className="flex items-center gap-2 min-w-0">
          <StatusDot tone={dotTone} pulse={running} className="shrink-0" />
          <span className="font-mono text-[13px] text-ink truncate">
            {pending ? (
              <>
                <span>permission to run</span>{' '}
              </>
            ) : running ? (
              <>running </>
            ) : (
              <>ran </>
            )}
            <span className="text-accent italic font-semibold">ƒ</span>{' '}
            <span className="text-ink">{message.functionId}</span>
            {!pending && !running && typeof message.durationMs === 'number' ? (
              <span className="text-ink-faint">
                {' '}
                for <span className="tabular-nums">{message.durationMs}</span>
                ms
              </span>
            ) : null}
          </span>
        </span>
        <span
          aria-hidden
          className={cn(
            'text-ink-ghost shrink-0 transition-transform duration-150 inline-block',
            open && 'rotate-90',
          )}
        >
          ▸
        </span>
      </button>

      {open ? (
        <div className="border-t border-rule-2">
          <ValuePane label="request" value={message.input} />
          {!pending && !running ? (
            <ValuePane label="response" value={message.output} bordered />
          ) : null}
        </div>
      ) : null}

      {pending ? (
        <div className="border-t border-rule-2 px-3 py-2 flex items-center gap-2">
          <Button variant="primary" size="sm" onClick={onApprove}>
            approve
          </Button>
          <Button variant="pill" size="sm" onClick={onDeny}>
            deny
          </Button>
        </div>
      ) : null}
    </div>
  )
}

interface ValuePaneProps {
  label: string
  value: unknown
  bordered?: boolean
}

function ValuePane({ label, value, bordered }: ValuePaneProps) {
  const empty = isEmptyValue(value)
  const primitive = !empty && isPrimitive(value)
  const single = !empty && !primitive ? singlePrimitiveField(value) : null

  if (empty) {
    return (
      <div className={cn(bordered && 'border-t border-rule-2')}>
        <div className="bg-paper-2 px-3 py-1.5 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          {label}
          <span className="text-ink-ghost normal-case tracking-normal">
            {' '}
            · empty
          </span>
        </div>
      </div>
    )
  }

  return (
    <div className={cn(bordered && 'border-t border-rule-2')}>
      <div className="bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
        {single ? (
          <span className="text-ink-ghost normal-case tracking-normal">
            {' '}
            · {single.key}
          </span>
        ) : null}
      </div>
      {primitive ? (
        <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
          <code>{formatPrimitive(value)}</code>
        </pre>
      ) : single ? (
        <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
          <code>{formatPrimitive(single.value)}</code>
        </pre>
      ) : (
        <JsonHighlight code={formatJson(value)} />
      )}
    </div>
  )
}
