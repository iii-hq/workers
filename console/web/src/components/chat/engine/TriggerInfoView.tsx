import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import { SchemaSection } from './FunctionInfoView'
import {
  safeParseRequest,
  type TriggerTypeDetail,
  triggerInfoRequestSchema,
} from './parsers'

interface TriggerInfoViewProps {
  input: unknown
  /** Parsed detail — the caller (index.tsx) falls back to panes on null. */
  detail?: TriggerTypeDetail
  running?: boolean
}

/**
 * `engine::triggers::info` — one trigger type's contract: owning worker,
 * live registration count, the per-binding config schema, and the event
 * payload it delivers. Mirrors `FunctionInfoView`'s layout so the two
 * inspector cards read as the same instrument.
 */
export function TriggerInfoView({
  input,
  detail,
  running,
}: TriggerInfoViewProps) {
  const req = safeParseRequest(triggerInfoRequestSchema, input)

  if (running || !detail) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? (
            <Chip>
              <span className="text-ink-faint uppercase tracking-[0.06em]">
                trigger
              </span>
              <span className="ml-1 text-ink break-all">{req.id}</span>
            </Chip>
          ) : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · inspecting trigger type…
        </div>
      </div>
    )
  }

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="trigger type" variant="accent" />
        <Chip>
          <span className="text-ink-faint uppercase tracking-[0.06em]">
            worker
          </span>
          <span className="ml-1 text-ink">{detail.worker_name}</span>
        </Chip>
        {typeof detail.instance_count === 'number' ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              registered
            </span>
            <span className="ml-1 text-ink tabular-nums">
              {detail.instance_count}
            </span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="⚡" tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {detail.id}
        </span>
      </ActionLine>
      {detail.description ? (
        <div className="px-3 py-2 border-b border-rule-2 font-mono text-[12px] text-ink-faint leading-[1.55]">
          {detail.description}
        </div>
      ) : null}
      <SchemaSection
        label="binding config"
        schema={detail.configuration_schema}
      />
      <SchemaSection label="event payload" schema={detail.request_schema} />
    </div>
  )
}
