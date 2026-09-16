import { ActionLine, Badge, Card, EmptyState, MarkdownPreview, MetaRow } from '@iii-dev/console-ui'
import { SquareFunction } from 'lucide-react'
import { ago } from '../lib/format'
import { Identity, kv, Loading, Row, Rows } from '../lib/widgets'
import {
  safeParseRequest,
  safeParseResponse,
  systemPromptsGetRequestSchema,
  systemPromptsGetResponseSchema,
  systemPromptsListResponseSchema,
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
          <Badge>listing…</Badge>
        </MetaRow>
        <Loading label="scanning system prompts folder…" />
      </Card>
    )
  }

  const resp = safeParseResponse(systemPromptsListResponseSchema, output)
  if (!resp) return null
  const n = resp.prompts.length

  return (
    <Card>
      <MetaRow>
        <Badge variant={n === 0 ? 'warn' : 'accent'}>
          {n === 0 ? 'no system prompts' : `${n} ${n === 1 ? 'system prompt' : 'system prompts'}`}
        </Badge>
      </MetaRow>
      {n === 0 ? (
        <EmptyState title="No system prompts" description="The system-prompts folder is empty." />
      ) : (
        <Rows>
          {resp.prompts.map((p) => (
            <Row key={p.name} mono title={p.name} description={p.description || undefined} meta={ago(p.modified_at)} />
          ))}
        </Rows>
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
        <MetaRow items={kv([['name', req?.name]])}>
          <Badge>loading…</Badge>
        </MetaRow>
        <Loading label="fetching system prompt…" />
      </Card>
    )
  }

  const resp = safeParseResponse(systemPromptsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow items={kv([['modified', ago(resp.modified_at)]])}>
        <Badge variant="accent">system prompt</Badge>
      </MetaRow>
      <ActionLine icon={<SquareFunction />}>
        <Identity name={resp.name} description={resp.description} />
      </ActionLine>
      <MarkdownPreview markdown={resp.body} />
    </Card>
  )
}
