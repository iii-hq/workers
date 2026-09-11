import {
  ActionLine,
  Chip,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  safeParseRequest,
  safeParseResponse,
  workersRegisterRequestSchema,
  workersRegisterResponseSchema,
} from './parsers'

interface WorkersRegisterViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function WorkersRegisterView({
  input,
  output,
  running,
}: WorkersRegisterViewProps) {
  const req = safeParseRequest(workersRegisterRequestSchema, input)
  if (!req) return null

  const headerLabel = running
    ? 'registering…'
    : (() => {
        const resp = safeParseResponse(workersRegisterResponseSchema, output)
        if (resp == null) return 'register failed'
        return resp.success ? 'registered' : 'register failed'
      })()
  const headerVariant: 'accent' | 'warn' | 'default' = running
    ? 'default'
    : headerLabel === 'registered'
      ? 'accent'
      : 'warn'

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label={headerLabel} variant={headerVariant} />
        {req.runtime ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              runtime
            </span>
            <span className="ml-1 text-ink">{req.runtime}</span>
          </Chip>
        ) : null}
        {req.version ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              version
            </span>
            <span className="ml-1 text-ink">{req.version}</span>
          </Chip>
        ) : null}
        {req.os ? (
          <Chip>
            <span className="text-ink-faint uppercase tracking-[0.06em]">
              os
            </span>
            <span className="ml-1 text-ink">{req.os}</span>
          </Chip>
        ) : null}
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <span className="font-mono text-[13px] text-accent break-all">
          {req.name ?? req._caller_worker_id}
        </span>
      </ActionLine>
      <div className="px-3 py-1.5 border-b border-rule-2 bg-paper-2 font-mono text-[11px] text-ink-faint flex flex-wrap gap-x-3 gap-y-0.5">
        <span title={req._caller_worker_id}>
          id: {shortenId(req._caller_worker_id)}
        </span>
        {req.telemetry?.install_kind ? (
          <span>install: {req.telemetry.install_kind}</span>
        ) : null}
        {req.telemetry?.device_id ? (
          <span title={req.telemetry.device_id}>
            device: {shortenId(req.telemetry.device_id)}
          </span>
        ) : null}
      </div>
    </div>
  )
}

function shortenId(id: string): string {
  if (id.length <= 14) return id
  return `${id.slice(0, 8)}…${id.slice(-4)}`
}
