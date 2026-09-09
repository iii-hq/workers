import { MarkdownPreview } from '@iii-dev/console-ui'
import { formatBytes, formatRelativeTime } from '../lib/format'
import { ActionLine, Card, EmptyRow, KvChip, MetaRow, PulseLine, StatusPill } from '../lib/widgets'
import {
  agentsFunctionsRequestSchema,
  agentsFunctionsResponseSchema,
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
                {a.skill_count != null ? `${a.skill_count} skills · ` : 'no skills · '}
                {a.function_count ? `${a.function_count} preloaded functions · ` : ''}
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
        {resp.functions && resp.functions.length > 0 ? (
          <KvChip label="preloaded functions">{resp.functions.join(', ')}</KvChip>
        ) : null}
        {resp.unknown_functions && resp.unknown_functions.length > 0 ? (
          <KvChip label="unknown functions">{resp.unknown_functions.join(', ')}</KvChip>
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

/* ---------------- directory::agents::functions::add / remove ---------------- */

export function AgentsFunctionsView({ input, output, running, verb }: ViewProps & { verb: 'add' | 'remove' }) {
  const req = safeParseRequest(agentsFunctionsRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label={verb === 'add' ? 'adding…' : 'removing…'} variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
          {req && req.functions.length > 0 ? <KvChip label="functions">{req.functions.join(', ')}</KvChip> : null}
        </MetaRow>
        <PulseLine label="rewriting the profile's preloaded functions…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsFunctionsResponseSchema, output)
  if (!resp) return null

  const changed = verb === 'add' ? (resp.added ?? []) : (resp.removed ?? [])
  return (
    <Card>
      <MetaRow>
        <StatusPill
          label={resp.unchanged ? 'unchanged' : verb === 'add' ? 'functions added' : 'functions removed'}
          variant={resp.unchanged ? 'default' : 'accent'}
        />
        <KvChip label="id">{resp.id}</KvChip>
        <KvChip label="bytes">{formatBytes(resp.bytes)}</KvChip>
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol={verb === 'add' ? '+' : '−'} tone="accent">
        <div className="dir-ui-stack">
          {changed.length > 0 ? (
            <span className="dir-ui-id">{changed.join(', ')}</span>
          ) : (
            <span className="dir-ui-desc">
              {verb === 'add' ? 'Every id was already present.' : 'None of the ids were present.'}
            </span>
          )}
          <span className="dir-ui-fine">
            {resp.functions.length === 0
              ? 'The profile now declares no preloaded functions.'
              : `now: ${resp.functions.join(', ')}`}
          </span>
        </div>
      </ActionLine>
    </Card>
  )
}
