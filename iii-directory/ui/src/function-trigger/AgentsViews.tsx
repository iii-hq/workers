import { MarkdownPreview } from '@iii-dev/console-ui'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { ActionLine, Card, EmptyRow, KvChip, MetaRow, PulseLine, StatusPill } from '../lib/widgets'
import {
  agentsGetRequestSchema,
  agentsGetResponseSchema,
  agentsListResponseSchema,
  agentsUpdateRequestSchema,
  agentsUpdateResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::agents::list ---------------- */

export function AgentsListView({ output, running }: ViewProps) {
  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
        </MetaRow>
        <PulseLine label="scanning agent profiles…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsListResponseSchema, output)
  if (!resp) return null

  const label =
    resp.agents.length === 0
      ? 'no agent profiles'
      : `${resp.agents.length} ${resp.agents.length === 1 ? 'agent profile' : 'agent profiles'}`

  return (
    <Card>
      <MetaRow>
        <StatusPill label={label} variant={resp.agents.length === 0 ? 'warn' : 'accent'} />
      </MetaRow>
      {resp.agents.length === 0 ? (
        <EmptyRow label="no agent profiles found" />
      ) : (
        <ul className="dir-ui-list">
          {resp.agents.map((a) => (
            <li key={a.id} className="dir-ui-row">
              <span className="dir-ui-id">{[a.logo, a.name || a.id].filter(Boolean).join(' ')}</span>
              {a.description ? <div className="dir-ui-desc">{a.description}</div> : null}
              <span className="dir-ui-fine">
                {a.skill_count != null ? `${a.skill_count} skills · ` : 'all skills · '}
                {formatRelativeTime(a.modified_at)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  )
}

/* ---------------- directory::agents::get ---------------- */

export function AgentsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(agentsGetRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
        </MetaRow>
        <PulseLine label="fetching agent profile…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label="agent profile" variant="accent" />
        {resp.model ? <KvChip label="model">{resp.model}</KvChip> : null}
        <KvChip label="skills">{resp.skills.length === 0 ? 'all' : String(resp.skills.length)}</KvChip>
        {resp.unknown_skills.length > 0 ? (
          <KvChip label="unknown skills">{resp.unknown_skills.join(', ')}</KvChip>
        ) : null}
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{[resp.logo, resp.name || resp.id].filter(Boolean).join(' ')}</span>
          {resp.description ? <span className="dir-ui-desc">{resp.description}</span> : null}
        </div>
      </ActionLine>
      <MarkdownPreview markdown={resp.system_prompt} />
    </Card>
  )
}

/* ---------------- directory::agents::update / create ---------------- */

export function AgentsUpdateView({ input, output, running, verb = 'updated' }: ViewProps & { verb?: string }) {
  const req = safeParseRequest(agentsUpdateRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="saving…" variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
        </MetaRow>
        <PulseLine label="writing agent profile…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label={verb} variant="accent" />
        <KvChip label="bytes">{formatBytes(resp.bytes)}</KvChip>
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="✎" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{[resp.logo, resp.name || resp.id].filter(Boolean).join(' ')}</span>
          {resp.description ? <span className="dir-ui-desc">{resp.description}</span> : null}
        </div>
      </ActionLine>
    </Card>
  )
}
