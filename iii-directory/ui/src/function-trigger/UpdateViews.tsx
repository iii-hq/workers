import { ActionLine, Badge, Card, MetaRow } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Pencil } from 'lucide-react'
import { ago } from '../lib/format'
import { Identity, kv, Loading } from '../lib/widgets'
import {
  safeParseRequest,
  safeParseResponse,
  skillsUpdateRequestSchema,
  skillsUpdateResponseSchema,
  systemPromptsUpdateRequestSchema,
  systemPromptsUpdateResponseSchema,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::skills::update ---------------- */

export function SkillsUpdateView({ input, output, running, verb = 'updated' }: ViewProps & { verb?: string }) {
  const req = safeParseRequest(skillsUpdateRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow items={kv([['id', req?.id]])}>
          <Badge>saving…</Badge>
        </MetaRow>
        <Loading label="writing skill…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['type', resp.type],
          ['bytes', formatBytes(resp.bytes)],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant="accent">{verb}</Badge>
      </MetaRow>
      <ActionLine icon={<Pencil />}>
        <Identity name={resp.id} description={resp.title} />
      </ActionLine>
    </Card>
  )
}

/* ---------------- directory::system-prompts::update ---------------- */

export function SystemPromptsUpdateView({ input, output, running, verb = 'updated' }: ViewProps & { verb?: string }) {
  const req = safeParseRequest(systemPromptsUpdateRequestSchema, input)

  if (running) {
    return (
      <Card>
        <MetaRow items={kv([['name', req?.name]])}>
          <Badge>saving…</Badge>
        </MetaRow>
        <Loading label="writing system prompt…" />
      </Card>
    )
  }

  const resp = safeParseResponse(systemPromptsUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['was', req && req.name !== resp.name && req.name],
          ['bytes', formatBytes(resp.bytes)],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant="accent">{verb}</Badge>
      </MetaRow>
      <ActionLine icon={<Pencil />}>
        <Identity name={resp.name} description={resp.description} />
      </ActionLine>
    </Card>
  )
}
