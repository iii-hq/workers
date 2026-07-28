import type { ReactNode } from 'react'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  configFilters,
  describeCron,
  type RegisterTriggerRequest,
  type RegisterTriggerResponse,
  registerTriggerRequestSchema,
  registerTriggerResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { FilterChip } from './shared'

interface RegisterTriggerViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/**
 * A trigger registration is a cause→effect rule: WHEN an event fires (filtered
 * by `config`) THEN deliver (notify the session, or call a plain function —
 * a binding never starts an agent). The view reads that way — a labeled
 * `when` block (the event + its filter chips) above a `then` block — so a
 * binding's meaning is legible at a glance. Raw payloads stay one tab away in
 * RAW JSON; this view is the readable one.
 */
export function RegisterTriggerView({
  input,
  output,
  running,
}: RegisterTriggerViewProps) {
  const req = safeParseRequest<RegisterTriggerRequest>(
    registerTriggerRequestSchema,
    input,
  )
  // Never render blank: an unrecognized payload falls back to raw JSON rather
  // than an empty terminal pane (the switch always mounts this component).
  if (!req) return <LabeledJson label="request" value={input} />

  const resp = running
    ? null
    : safeParseResponse<RegisterTriggerResponse>(
        registerTriggerResponseSchema,
        output,
      )
  const regId = resp?.id ?? resp?.subscription_id
  const once = resp?.once ?? req.once

  // Cron schedules read as WHEN content, not as an opaque config dump: the
  // expression chip plus (for the common shapes) a human reading of it.
  const cronExpression =
    req.trigger_type === 'cron' &&
    req.config &&
    typeof req.config === 'object' &&
    typeof (req.config as { expression?: unknown }).expression === 'string'
      ? (req.config as { expression: string }).expression
      : null
  const cronHuman = cronExpression ? describeCron(cronExpression) : null
  const cronOnlyConfig =
    cronExpression != null &&
    Object.keys(req.config as Record<string, unknown>).length === 1

  const filters = configFilters(req.config)
  const showRawConfig = !filters && !isEmpty(req.config) && !cronOnlyConfig

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={running ? 'registering trigger…' : 'trigger registered'}
          variant={running ? 'default' : 'accent'}
        />
        {req.label ? <FilterChip label="label" value={req.label} /> : null}
        {typeof once === 'boolean' ? (
          <FilterChip label="mode" value={once ? 'one-shot' : 'persistent'} />
        ) : null}
        {regId ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              id
            </span>
            <span className="ml-1 text-ink" title={regId}>
              {shortenId(regId)}
            </span>
          </Chip>
        ) : null}
      </MetaRow>

      <PaneLabel>when</PaneLabel>
      <div className="px-3 py-2 border-b border-rule-2 bg-bg flex flex-col gap-1.5">
        <span className="font-mono text-[13px] text-ink break-all">
          {req.trigger_type}
        </span>
        {cronExpression ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {cronHuman ? (
              <span className="font-mono text-[12px] text-ink">
                {cronHuman}
              </span>
            ) : null}
            <FilterChip label="expression" value={cronExpression} />
          </div>
        ) : filters ? (
          <div className="flex flex-wrap items-center gap-1.5">
            {filters.map((f) => (
              <FilterChip key={f.label} label={f.label} value={f.value} />
            ))}
          </div>
        ) : showRawConfig ? null : (
          <span className="font-mono text-[11px] text-ink-ghost">
            · no filter — fires on every event
          </span>
        )}
      </div>
      {showRawConfig ? <LabeledJson label="config" value={req.config} /> : null}

      <PaneLabel>then</PaneLabel>
      <div className="px-3 py-2 border-b border-rule-2 bg-bg flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2 flex-wrap">
          <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            {req.function_id ? 'call' : 'notify'}
          </span>
          {req.function_id ? (
            <span className="font-mono text-[12.5px] text-accent break-all">
              {req.function_id}
            </span>
          ) : (
            <span className="font-mono text-[12.5px] text-ink-faint italic">
              this session
            </span>
          )}
        </div>
      </div>

      {req.metadata !== undefined && !isEmpty(req.metadata) ? (
        <LabeledJson label="metadata" value={req.metadata} />
      ) : null}
    </div>
  )
}

function isEmpty(v: unknown): boolean {
  if (v === null || v === undefined) return true
  if (typeof v === 'object') {
    return Object.keys(v as Record<string, unknown>).length === 0
  }
  return false
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}

function PaneLabel({ children }: { children: ReactNode }) {
  return (
    <div className="bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
      {children}
    </div>
  )
}

function LabeledJson({ label, value }: { label: string; value: unknown }) {
  return (
    <div>
      <PaneLabel>{label}</PaneLabel>
      <JsonHighlight code={JSON.stringify(value ?? null, null, 2)} />
    </div>
  )
}
