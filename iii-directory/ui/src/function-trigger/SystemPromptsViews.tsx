import { MarkdownPreview } from '@iii-dev/console-ui'
import { formatRelativeTime } from '../lib/format'
import {
  ActionLine,
  Card,
  EmptyRow,
  KvChip,
  MetaRow,
  PulseLine,
  StatusPill,
} from '../lib/widgets'
import {
  systemPromptsGetRequestSchema,
  systemPromptsGetResponseSchema,
  systemPromptsListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::system-prompts::list ---------------- */

export function SystemPromptsListView({ output, running }: ViewProps) {
  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
        </MetaRow>
        <PulseLine label="scanning system prompts folder…" />
      </Card>
    )
  }

  const resp = safeParseResponse(systemPromptsListResponseSchema, output)
  if (!resp) return null

  const label =
    resp.prompts.length === 0
      ? 'no system prompts'
      : `${resp.prompts.length} ${
          resp.prompts.length === 1 ? 'system prompt' : 'system prompts'
        }`

  return (
    <Card>
      <MetaRow>
        <StatusPill
          label={label}
          variant={resp.prompts.length === 0 ? 'warn' : 'accent'}
        />
      </MetaRow>
      {resp.prompts.length === 0 ? (
        <EmptyRow label="no system prompts found" />
      ) : (
        <ul className="dir-ui-list">
          {resp.prompts.map((p) => (
            <li key={p.name} className="dir-ui-row">
              <span className="dir-ui-id">{p.name}</span>
              {p.description ? (
                <div className="dir-ui-desc">{p.description}</div>
              ) : null}
              <span className="dir-ui-fine">
                {formatRelativeTime(p.modified_at)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  )
}

/* ---------------- directory::system-prompts::get ---------------- */

export function SystemPromptsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(systemPromptsGetRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="name">{req.name}</KvChip> : null}
        </MetaRow>
        <PulseLine label="fetching system prompt…" />
      </Card>
    )
  }

  const resp = safeParseResponse(systemPromptsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label="system prompt" variant="accent" />
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="dir-ui-stack">
          <span className="dir-ui-id lg">{resp.name}</span>
          {resp.description ? (
            <span className="dir-ui-desc">{resp.description}</span>
          ) : null}
        </div>
      </ActionLine>
      <MarkdownPreview markdown={resp.body} />
    </Card>
  )
}
