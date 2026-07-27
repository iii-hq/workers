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
  promptsGetRequestSchema,
  promptsGetResponseSchema,
  promptsListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::prompts::list ---------------- */

export function PromptsListView({ output, running }: ViewProps) {
  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
        </MetaRow>
        <PulseLine label="scanning prompts folder…" />
      </Card>
    )
  }

  const resp = safeParseResponse(promptsListResponseSchema, output)
  if (!resp) return null

  const label =
    resp.prompts.length === 0
      ? 'no prompts'
      : `${resp.prompts.length} ${
          resp.prompts.length === 1 ? 'prompt' : 'prompts'
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
        <EmptyRow label="no prompts found" />
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

/* ---------------- directory::prompts::get ---------------- */

export function PromptsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(promptsGetRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="name">{req.name}</KvChip> : null}
        </MetaRow>
        <PulseLine label="fetching prompt…" />
      </Card>
    )
  }

  const resp = safeParseResponse(promptsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow>
        <StatusPill label="prompt" variant="accent" />
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
