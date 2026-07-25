import type { ReactNode } from 'react'
import { PipeStepList } from '@/components/chat/fp/PipeView'
import {
  pipeRequestSchema,
  safeParseRequest as safeParseFpRequest,
} from '@/components/chat/fp/parsers'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  configFilters,
  describeCron,
  type ReactOptions,
  type ReactSpec,
  type RegisterTriggerRequest,
  type RegisterTriggerResponse,
  reactOptionsSchema,
  reactSpecSchema,
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
 * by `config`) THEN run an action (`function_id` + `metadata`). The view reads
 * that way — a labeled `when` block (the event + its filter chips) above a
 * `then` block (the action: spawn a sub-agent / notify the session / call a
 * function) — so a binding's meaning is legible at a glance. Raw payloads stay
 * one tab away in RAW JSON; this view is the readable one.
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

  // A `harness::react` binding runs in one of two modes: spawn a sub-agent
  // (`task`, + `model`) or dispatch a plain function call (`call`). The
  // non-strict schema parses ANY object, so the mode fields decide whether
  // this is really a react spec; neither present → raw metadata fallback.
  const parsedReact =
    req.function_id === 'harness::react'
      ? safeParseRequest<ReactSpec>(reactSpecSchema, req.metadata)
      : null
  const react = parsedReact?.task || parsedReact?.call ? parsedReact : null
  const call = react?.call ?? null
  const spawn = call ? null : react
  // A call-mode target that is itself a pipe renders as its step route —
  // the whole point of the binding is legible as event → pipeline.
  const callPipe =
    call?.function_id === 'fp::pipe'
      ? (safeParseFpRequest(pipeRequestSchema, call.payload)?.through ?? null)
      : null
  const allow = spawn
    ? safeParseRequest<ReactOptions>(reactOptionsSchema, spawn.options)
        ?.functions?.allow
    : undefined
  const allowUniq = allow ? Array.from(new Set(allow)) : []

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
            {spawn
              ? 'spawn sub-agent'
              : call || req.function_id
                ? 'call'
                : 'notify'}
          </span>
          {spawn ? (
            <span className="font-mono text-[12.5px] text-accent break-all">
              {spawn.model ?? 'inherited model'}
            </span>
          ) : call ? (
            // The binding's real effect is the inner target, not the
            // harness::react plumbing it rides on.
            <span className="font-mono text-[12.5px] text-accent break-all">
              {call.function_id}
            </span>
          ) : req.function_id ? (
            <span className="font-mono text-[12.5px] text-accent break-all">
              {req.function_id}
            </span>
          ) : (
            <span className="font-mono text-[12.5px] text-ink-faint italic">
              this session
            </span>
          )}
          {call?.event_into ? (
            <FilterChip label="event into" value={call.event_into} />
          ) : null}
          {spawn?.session_id ? (
            <>
              <span className="font-mono text-[11px] text-ink-ghost">→</span>
              <span
                className="font-mono text-[11.5px] text-ink-faint"
                title={spawn.session_id}
              >
                {shortenId(spawn.session_id)}
              </span>
            </>
          ) : null}
        </div>
        {allowUniq.length ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
              allow
            </span>
            {allowUniq.map((fn) => (
              <Chip key={fn}>
                <span className="text-ink">{fn}</span>
              </Chip>
            ))}
          </div>
        ) : null}
        {react?.join ? (
          <div className="font-mono text-[12px] text-ink flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="text-ink-faint uppercase tracking-[0.06em] text-[10px]">
              join
            </span>
            <span className="text-accent break-all">{react.join.id}</span>
            <span className="text-ink-ghost">·</span>
            <span>
              key <span className="text-ink-faint">{react.join.key}</span>
            </span>
            <span className="text-ink-ghost">·</span>
            <span>
              expect{' '}
              <span className="text-ink-faint">
                [{react.join.expect.join(', ')}]
              </span>
            </span>
            {react.join.rearm ? (
              <>
                <span className="text-ink-ghost">·</span>
                <span className="text-accent">rearm</span>
              </>
            ) : null}
          </div>
        ) : null}
      </div>

      {callPipe?.length ? (
        <div>
          <PaneLabel>pipeline</PaneLabel>
          <PipeStepList through={callPipe} />
        </div>
      ) : call?.payload !== undefined && !isEmpty(call.payload) ? (
        <LabeledJson label="payload" value={call.payload} />
      ) : spawn?.task ? (
        <LabeledText label="task" text={spawn.task} />
      ) : !react && req.metadata !== undefined && !isEmpty(req.metadata) ? (
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

function LabeledText({ label, text }: { label: string; text: string }) {
  return (
    <div>
      <PaneLabel>{label}</PaneLabel>
      <pre className="bg-bg overflow-auto max-h-60 px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
        <code>{text}</code>
      </pre>
    </div>
  )
}
