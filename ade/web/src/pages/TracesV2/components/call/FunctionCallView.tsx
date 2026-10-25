/**
 * The span panel's view of one function invocation that is NOT a model
 * call (`compose::status`, `state::get`, a worker's `execute <fn>` …):
 * the same vocabulary as the model-call view — a facts strip, then the
 * captured request and response as padded, highlighted panes with copy
 * and wrap — so the two read as siblings instead of the chat's edge-to-
 * edge card dropped into the panel.
 *
 * A renderer that claims the function (a worker's injected UI, the
 * first-party families) still gets its say: its node renders in a
 * "Rendered" section above the raw panes.
 */

import { AlertCircle, SquareFunction } from 'lucide-react'
import { type ReactNode, useMemo } from 'react'
import {
  firstRendered,
  useFunctionTriggerRenderers,
} from '@/components/function-trigger/renderer-registry'
import { cn } from '@/lib/utils'
import type { FunctionTriggerMessage } from '@/types/chat'
import { parsePayload, truncatedBytesOf } from '../../lib/llmCall'
import { formatDuration } from '../../lib/traceUtils'
import { PayloadPane } from '../llm/PayloadPane'

export interface FunctionCallViewProps {
  message: FunctionTriggerMessage
  /** The request/response after the function's redactor ran. */
  rawInput: unknown
  rawOutput: unknown
  worker: string
  durationMs: number
  status: 'ok' | 'error' | 'pending' | 'unset' | string
}

function Fact({
  label,
  value,
  mono = true,
  tone,
  title,
  className,
}: {
  label: string
  value: ReactNode
  mono?: boolean
  tone?: 'alert' | 'ok'
  title?: string
  className?: string
}) {
  return (
    <div
      className={cn('flex min-w-0 flex-col gap-0.5', className)}
      title={title}
    >
      <span className="text-[11px] text-muted-foreground">{label}</span>
      <span
        className={cn(
          'truncate text-[13px] text-foreground tabular-nums',
          mono && 'font-mono text-[13px]',
          tone === 'alert' && 'text-alert-strong',
          tone === 'ok' && 'text-ok',
        )}
      >
        {value}
      </span>
    </div>
  )
}

function isEmpty(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'string') return v.trim().length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') return Object.keys(v as object).length === 0
  return false
}

/** `{ error: … }` soft failures ride on the output (see FunctionTriggerCard). */
function errorTextOf(output: unknown): string | null {
  if (!output || typeof output !== 'object' || Array.isArray(output))
    return null
  const e = (output as { error?: unknown }).error
  if (e === undefined) return null
  if (typeof e === 'string') return e
  if (e && typeof e === 'object') {
    const o = e as { kind?: unknown; message?: unknown }
    const parts = [o.kind, o.message].filter(
      (x): x is string => typeof x === 'string',
    )
    return parts.length ? parts.join(': ') : JSON.stringify(e)
  }
  return String(e)
}

export function FunctionCallView({
  message,
  rawInput,
  rawOutput,
  worker,
  durationMs,
  status,
}: FunctionCallViewProps) {
  const renderers = useFunctionTriggerRenderers()
  const rendered = useMemo(
    () =>
      message.running
        ? null
        : firstRendered(renderers, (r) =>
            r.isMatch(message.functionId) ? r.tryRender(message) : null,
          ),
    [renderers, message],
  )
  const request = useMemo(() => parsePayload(rawInput), [rawInput])
  const response = useMemo(() => parsePayload(rawOutput), [rawOutput])
  const errorText = errorTextOf(response)
  const failed = status === 'error' || errorText !== null
  const requestKeys =
    request && typeof request === 'object' && !Array.isArray(request)
      ? Object.keys(request as object).filter((k) => k !== '_caller_worker_id')
      : []

  return (
    <div className="flex flex-col gap-4" data-function-call-view>
      <div className="grid grid-cols-2 gap-x-4 gap-y-3 rounded-lg border border-border bg-muted/30 px-3 py-2.5 @md:grid-cols-3">
        <div className="col-span-2 flex min-w-0 flex-col gap-0.5 @md:col-span-3">
          <span className="text-[11px] text-muted-foreground">Function</span>
          <span className="flex min-w-0 items-center gap-2">
            <SquareFunction
              className="size-4 shrink-0 text-muted-foreground"
              aria-hidden
            />
            <span className="truncate font-mono text-[13px] font-medium text-foreground">
              {message.functionId}
            </span>
            <span className="shrink-0 rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground">
              {worker}
            </span>
            {message.identityInherited ? (
              <span
                className="shrink-0 text-[11px] text-muted-foreground"
                title="this span runs inside the function's invocation; its timing is the sub-span's, not the call's"
              >
                inside the call
              </span>
            ) : null}
          </span>
        </div>
        <Fact
          label="Status"
          value={failed ? 'error' : status === 'pending' ? 'running' : status}
          tone={failed ? 'alert' : status === 'ok' ? 'ok' : undefined}
        />
        <Fact label="Duration" value={formatDuration(durationMs)} />
        <Fact
          label="Arguments"
          value={
            requestKeys.length === 0
              ? isEmpty(request)
                ? 'none'
                : '1 value'
              : `${requestKeys.length} ${requestKeys.length === 1 ? 'key' : 'keys'}`
          }
          title={requestKeys.join(', ') || undefined}
        />
      </div>

      {errorText ? (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-lg border border-alert/30 bg-alert-muted px-3 py-2 text-[13px] text-foreground"
        >
          <AlertCircle
            className="mt-0.5 size-4 shrink-0 text-alert"
            aria-hidden
          />
          <span className="min-w-0 break-words whitespace-pre-wrap">
            {errorText}
          </span>
        </div>
      ) : null}

      {rendered ? (
        <section className="flex flex-col gap-2">
          <h4 className="text-xs font-semibold text-foreground">
            {rendered.renderer.primaryTabLabel ?? 'Rendered'}
          </h4>
          <div className="overflow-hidden rounded-lg border border-border bg-background p-3 function-trigger-surface">
            {rendered.node}
          </div>
        </section>
      ) : null}

      <PayloadPane
        label="request"
        value={isEmpty(request) ? {} : request}
        hint={isEmpty(request) ? 'empty' : undefined}
        truncatedBytes={truncatedBytesOf(rawInput)}
      />
      <PayloadPane
        label="response"
        value={
          message.running
            ? 'still running — the response lands when the call completes'
            : isEmpty(response)
              ? {}
              : response
        }
        language="text"
        hint={!message.running && isEmpty(response) ? 'empty' : undefined}
        truncatedBytes={truncatedBytesOf(rawOutput)}
      />
    </div>
  )
}
