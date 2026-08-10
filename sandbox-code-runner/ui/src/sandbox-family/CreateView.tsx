/**
 * `sandbox::create` — the accent slab. Ported from the console's
 * CreateView; the minted sandbox id is now the interactive chip
 * (copy + jump to the fleet page) instead of a passive code block.
 */

import { normaliseEnv } from './format'
import { createRequestSchema, createResponseSchema, safeParseResponse } from './parsers'
import { Chip, SandboxIdChip } from './shared'

interface CreateViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function CreateView({ input, output, running }: CreateViewProps) {
  const req = createRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData = output != null ? safeParseResponse(createResponseSchema, output) : null
  const env = normaliseEnv(req.data.env)

  return (
    <div className="cr-fam-card">
      <div className="cr-fam-slab accent">
        <div className="cr-fam-line">
          <span className="cr-fam-accent">+</span>
          <span>{running ? 'creating sandbox…' : 'created sandbox'}</span>
          {respData ? <SandboxIdChip sandboxId={respData.sandbox_id} /> : null}
        </div>
        <div className="cr-fam-chips">
          <Chip label="image">{respData?.image ?? req.data.image}</Chip>
          {typeof req.data.cpus === 'number' ? <Chip label="cpus">{req.data.cpus}</Chip> : null}
          {typeof req.data.memory_mb === 'number' ? <Chip label="mem">{`${req.data.memory_mb} MiB`}</Chip> : null}
          {req.data.name ? <Chip label="name">{req.data.name}</Chip> : null}
          {typeof req.data.network === 'boolean' ? <Chip label="net">{req.data.network ? 'on' : 'off'}</Chip> : null}
          {typeof req.data.idle_timeout_secs === 'number' ? (
            <Chip label="idle">{`${req.data.idle_timeout_secs}s`}</Chip>
          ) : null}
        </div>
        {env.length > 0 ? (
          <div className="cr-fam-chips">
            <span className="cr-fam-env-k">env</span>
            {/* Names only — values are masked so secrets never land in
                transcripts. */}
            {env.map(([k]) => (
              <span key={k} className="cr-fam-env">
                <span className="strong">{k}</span>=••••
              </span>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  )
}
