import type { ReactNode } from 'react'
import { Chip, MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import { JsonHighlight } from '@/lib/syntax'
import {
  type ReactOptions,
  type ReactSpec,
  type RegisterTriggerRequest,
  type RegisterTriggerResponse,
  reactOptionsSchema,
  reactSpecSchema,
  registerTriggerRequestSchema,
  registerTriggerResponseSchema,
  type StateTriggerConfig,
  safeParseRequest,
  safeParseResponse,
  stateTriggerConfigSchema,
} from './parsers'
import { FilterChip } from './shared'

interface RegisterTriggerViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

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

  const stateCfg =
    req.trigger_type === 'state'
      ? safeParseRequest<StateTriggerConfig>(
          stateTriggerConfigSchema,
          req.config,
        )
      : null
  const react =
    req.function_id === 'harness::react'
      ? safeParseRequest<ReactSpec>(reactSpecSchema, req.metadata)
      : null
  const allow = react
    ? safeParseRequest<ReactOptions>(reactOptionsSchema, react.options)
        ?.functions?.allow
    : undefined

  const resp = running
    ? null
    : safeParseResponse<RegisterTriggerResponse>(
        registerTriggerResponseSchema,
        output,
      )
  const regId = resp?.id ?? resp?.subscription_id
  const once = resp?.once ?? req.once

  const hasStateChips =
    !!stateCfg &&
    (!!stateCfg.scope || !!stateCfg.key || !!stateCfg.condition_function_id)

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

      <div className="px-3 py-2 border-b border-rule-2 bg-bg flex items-baseline gap-2 flex-wrap">
        <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
          {req.trigger_type}
        </span>
        <span className="font-mono text-[11px] text-ink-faint">→</span>
        {req.function_id ? (
          <span className="font-mono text-[12.5px] text-accent break-all">
            {req.function_id}
          </span>
        ) : (
          <span className="font-mono text-[12.5px] text-ink-faint italic">
            notify session
          </span>
        )}
      </div>

      {hasStateChips ? (
        <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 flex flex-wrap items-center gap-1.5">
          {stateCfg?.scope ? (
            <FilterChip label="scope" value={stateCfg.scope} />
          ) : null}
          {stateCfg?.key ? (
            <FilterChip label="key" value={stateCfg.key} />
          ) : null}
          {stateCfg?.condition_function_id ? (
            <FilterChip label="if" value={stateCfg.condition_function_id} />
          ) : null}
        </div>
      ) : req.config !== undefined && !isEmpty(req.config) ? (
        <LabeledJson label="config" value={req.config} />
      ) : null}

      {react ? (
        <>
          <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 flex flex-wrap items-center gap-1.5">
            <FilterChip label="model" value={react.model} />
            {allow?.length
              ? Array.from(new Set(allow)).map((fn) => (
                  <Chip key={fn}>
                    <span className="text-ink">{fn}</span>
                  </Chip>
                ))
              : null}
          </div>
          {react.join ? (
            <div className="px-3 py-2 border-b border-rule-2 bg-bg font-mono text-[12px] text-ink flex flex-wrap items-center gap-x-2 gap-y-1">
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
          <LabeledText label="task" text={react.task} />
        </>
      ) : req.metadata !== undefined ? (
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
      <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
        <code>{text}</code>
      </pre>
    </div>
  )
}
