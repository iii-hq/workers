import { useState } from 'react'
import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { CodeHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import {
  type FunctionDetail,
  functionDetailSchema,
  functionInfoRequestSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface FunctionInfoViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function FunctionInfoView({
  input,
  output,
  running,
}: FunctionInfoViewProps) {
  const req = safeParseRequest(functionInfoRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? (
            <Chip>
              <span className="text-ink-faint uppercase tracking-[0.06em]">
                function
              </span>
              <span className="ml-1 text-ink break-all">{req.function_id}</span>
            </Chip>
          ) : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · inspecting function…
        </div>
      </div>
    )
  }

  const detail = safeParseResponse(functionDetailSchema, output)
  if (!detail) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="function" variant="accent" />
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            worker
          </span>
          <span className="ml-1 text-ink">{detail.worker_name}</span>
        </Chip>
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            triggers
          </span>
          <span className="ml-1 text-ink tabular-nums">
            {detail.registered_triggers.length}
          </span>
        </Chip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {detail.function_id}
        </span>
      </ActionLine>
      {detail.description ? (
        <div className="px-3 py-2 border-b border-rule-2 font-mono text-[12px] text-ink-faint leading-[1.55]">
          {detail.description}
        </div>
      ) : null}
      <SchemaSection label="request" schema={detail.request_schema} />
      <SchemaSection label="response" schema={detail.response_schema} />
      {detail.metadata !== undefined ? (
        <SchemaSection
          label="metadata"
          schema={detail.metadata}
          treatEmptyAsAny={false}
        />
      ) : null}
      <RegisteredTriggers triggers={detail.registered_triggers} />
    </div>
  )
}

function RegisteredTriggers({
  triggers,
}: {
  triggers: FunctionDetail['registered_triggers']
}) {
  if (triggers.length === 0) {
    return (
      <div className="px-3 py-2 border-b border-rule-2 font-mono text-[11.5px] text-ink-ghost">
        · no registered triggers
      </div>
    )
  }
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        registered triggers · {triggers.length}
      </div>
      <ul className="divide-y divide-rule-2">
        {triggers.map((t) => (
          <li key={t.id} className="px-3 py-2 flex flex-col gap-1">
            <div className="flex items-baseline gap-2 flex-wrap">
              <span
                className="font-mono text-[11px] text-ink-faint break-all"
                title={t.id}
              >
                {shortenId(t.id)}
              </span>
              <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                {t.trigger_type}
              </span>
            </div>
            <TriggerConfig config={t.config} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function TriggerConfig({ config }: { config: unknown }) {
  if (config === undefined || config === null) return null
  if (
    typeof config === 'object' &&
    Object.keys(config as object).length === 0
  ) {
    return (
      <div className="font-mono text-[11px] text-ink-ghost">· no config</div>
    )
  }
  let pretty: string
  try {
    pretty = JSON.stringify(config, null, 2)
  } catch {
    return null
  }
  return (
    <CodeHighlight
      code={pretty}
      language="json"
      wrap
      className="border border-rule-2 text-[11.5px] leading-[1.5] px-2 py-1.5"
    />
  )
}

interface SchemaSectionProps {
  label: string
  schema: unknown
  treatEmptyAsAny?: boolean
}

function SchemaSection({
  label,
  schema,
  treatEmptyAsAny = true,
}: SchemaSectionProps) {
  const [open, setOpen] = useState(false)
  if (schema === undefined || schema === null) {
    return (
      <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        {label} · none
      </div>
    )
  }
  const isAny = treatEmptyAsAny && isAnySchema(schema)
  const pretty = tryStringifyJson(schema)
  return (
    <div className="border-b border-rule-2 bg-paper-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full px-3 py-1.5 flex items-center justify-between gap-2 text-left hover:bg-paper-3 cursor-pointer"
        aria-expanded={open}
        disabled={pretty == null}
      >
        <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
          {label}
          {isAny ? (
            <span className="ml-1.5 text-ink-faint normal-case tracking-normal">
              · any
            </span>
          ) : null}
        </span>
        {pretty != null ? (
          <span
            aria-hidden
            className={cn(
              'text-ink-ghost transition-transform duration-150 inline-block text-[10px]',
              open && 'rotate-90',
            )}
          >
            ▸
          </span>
        ) : null}
      </button>
      {open && pretty != null ? (
        <CodeHighlight
          code={pretty}
          language="json"
          wrap
          className="border-t border-rule-2 text-[11.5px] leading-[1.5]"
        />
      ) : null}
    </div>
  )
}

function isAnySchema(value: unknown): boolean {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false
  const obj = value as Record<string, unknown>
  // JsonSchema's `AnyValue` placeholder ships `$schema` + `title: "AnyValue"`
  // with no constraints, which the harness wraps around `Value` request /
  // response payloads where the engine has no static schema.
  if (obj.title === 'AnyValue') return true
  const keys = Object.keys(obj)
  if (keys.length === 0) return true
  if (keys.every((k) => k === '$schema' || k === 'title')) return true
  return false
}

function tryStringifyJson(value: unknown): string | null {
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return null
  }
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}
