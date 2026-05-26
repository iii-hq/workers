import { normaliseEnv, truncateMiddle } from './format'
import {
  createRequestSchema,
  createResponseSchema,
  safeParseResponse,
} from './parsers'
import { Chip } from './terminal/Terminal'

interface CreateViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function CreateView({ input, output, running }: CreateViewProps) {
  const req = createRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(createResponseSchema, output) : null
  const env = normaliseEnv(req.data.env)

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2 border-l-2 border-accent">
        <div className="font-mono text-[12.5px] text-ink flex flex-wrap items-baseline gap-2">
          <span className="text-accent">+</span>
          <span>{running ? 'creating sandbox…' : 'created sandbox'}</span>
          {respData ? (
            <code className="bg-paper-2 border border-rule-2 px-1.5 py-0.5 text-[12px] text-ink">
              {truncateMiddle(respData.sandbox_id, 24)}
            </code>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="image">{respData?.image ?? req.data.image}</Chip>
          {typeof req.data.cpus === 'number' ? (
            <Chip label="cpus">{req.data.cpus}</Chip>
          ) : null}
          {typeof req.data.memory_mb === 'number' ? (
            <Chip label="mem">{`${req.data.memory_mb} MiB`}</Chip>
          ) : null}
          {req.data.name ? <Chip label="name">{req.data.name}</Chip> : null}
          {typeof req.data.network === 'boolean' ? (
            <Chip label="net">{req.data.network ? 'on' : 'off'}</Chip>
          ) : null}
          {typeof req.data.idle_timeout_secs === 'number' ? (
            <Chip label="idle">{`${req.data.idle_timeout_secs}s`}</Chip>
          ) : null}
        </div>
        {env.length > 0 ? (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
              env
            </span>
            {env.map(([k, v]) => (
              <span
                key={k}
                className="font-mono text-[11px] text-ink-faint border border-rule-2 bg-paper-2 px-1.5 py-0.5"
              >
                <span className="text-ink">{k}</span>={v}
              </span>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  )
}
