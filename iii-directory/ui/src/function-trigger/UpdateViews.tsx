import { formatBytes, formatRelativeTime } from '../lib/format'
import {
  ActionLine,
  Card,
  KvChip,
  MetaRow,
  PulseLine,
  StatusPill,
} from '../lib/widgets'
import {
  promptsUpdateRequestSchema,
  promptsUpdateResponseSchema,
  safeParseRequest,
  safeParseResponse,
  skillsUpdateRequestSchema,
  skillsUpdateResponseSchema,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::skills::update ---------------- */

export function SkillsUpdateView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsUpdateRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="saving…" variant="default" />
          {req ? <KvChip label="id">{req.id}</KvChip> : null}
        </MetaRow>
        <PulseLine label="writing skill…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label="updated" variant="accent" />
        {resp.type ? <KvChip label="type">{resp.type}</KvChip> : null}
        <KvChip label="bytes">{formatBytes(resp.bytes)}</KvChip>
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="✎" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{resp.id}</span>
          <span className="dir-ui-desc">{resp.title}</span>
        </div>
      </ActionLine>
    </Card>
  )
}

/* ---------------- directory::prompts::update ---------------- */

export function PromptsUpdateView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(promptsUpdateRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="saving…" variant="default" />
          {req ? <KvChip label="name">{req.name}</KvChip> : null}
        </MetaRow>
        <PulseLine label="writing prompt…" />
      </Card>
    )
  }

  const resp = safeParseResponse(promptsUpdateResponseSchema, output)
  if (!resp) return null

  const renamed = req && req.name !== resp.name

  return (
    <Card>
      <MetaRow>
        <StatusPill label="updated" variant="accent" />
        {renamed ? <KvChip label="was">{req.name}</KvChip> : null}
        <KvChip label="bytes">{formatBytes(resp.bytes)}</KvChip>
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="✎" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{resp.name}</span>
          {resp.description ? (
            <span className="dir-ui-desc">{resp.description}</span>
          ) : null}
        </div>
      </ActionLine>
    </Card>
  )
}
