import { ActionLine, Badge, Card, EmptyState, MarkdownPreview, MetaRow } from '@iii-dev/console-ui'
import { formatBytes } from '@iii-dev/console-ui/format'
import { Minus, Pencil, Plus, SquareFunction } from 'lucide-react'
import { ago } from '../lib/format'
import { Identity, kv, Loading, Row, Rows } from '../lib/widgets'
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

const displayName = (a: { logo?: string | null; name?: string | null; id: string }) =>
  [a.logo, a.name || a.id].filter(Boolean).join(' ')

/* ---------------- directory::agents::list ---------------- */

export function AgentsListView({ output, running }: ViewProps) {
  if (running) {
    return (
      <Card>
        <MetaRow>
          <Badge>listing…</Badge>
        </MetaRow>
        <Loading label="scanning agent profiles…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsListResponseSchema, output)
  if (!resp) return null
  const n = resp.agents.length

  return (
    <Card>
      <MetaRow>
        <Badge variant={n === 0 ? 'warn' : 'accent'}>
          {n === 0 ? 'no agent profiles' : `${n} ${n === 1 ? 'agent profile' : 'agent profiles'}`}
        </Badge>
      </MetaRow>
      {n === 0 ? (
        <EmptyState title="No agent profiles" description="The agents folder holds no profiles yet." />
      ) : (
        <Rows>
          {resp.agents.map((a) => (
            <Row
              key={a.id}
              title={displayName(a)}
              description={a.description || undefined}
              meta={[
                a.skill_count != null ? `${a.skill_count} skills` : 'no skills',
                a.function_count ? `${a.function_count} preloaded functions` : '',
                ago(a.modified_at),
              ]
                .filter(Boolean)
                .join(' · ')}
            />
          ))}
        </Rows>
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
        <MetaRow items={kv([['id', req?.id]])}>
          <Badge>loading…</Badge>
        </MetaRow>
        <Loading label="fetching agent profile…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsGetResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['model', resp.model],
          ['skills', resp.skills.length === 0 ? 'all' : String(resp.skills.length)],
          ['unknown skills', resp.unknown_skills.length > 0 && resp.unknown_skills.join(', ')],
          ['preloaded functions', resp.functions?.length ? resp.functions.join(', ') : null],
          ['unknown functions', resp.unknown_functions?.length ? resp.unknown_functions.join(', ') : null],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant="accent">agent profile</Badge>
      </MetaRow>
      <ActionLine icon={<SquareFunction />}>
        <Identity name={displayName(resp)} description={resp.description} />
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
        <MetaRow items={kv([['id', req?.id]])}>
          <Badge>saving…</Badge>
        </MetaRow>
        <Loading label="writing agent profile…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsUpdateResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['bytes', formatBytes(resp.bytes)],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant="accent">{verb}</Badge>
      </MetaRow>
      <ActionLine icon={<Pencil />}>
        <Identity name={displayName(resp)} description={resp.description} />
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
        <MetaRow
          items={kv([
            ['id', req?.id],
            ['functions', req && req.functions.length > 0 && req.functions.join(', ')],
          ])}
        >
          <Badge>{verb === 'add' ? 'adding…' : 'removing…'}</Badge>
        </MetaRow>
        <Loading label="rewriting the profile's preloaded functions…" />
      </Card>
    )
  }

  const resp = safeParseResponse(agentsFunctionsResponseSchema, output)
  if (!resp) return null

  const changed = verb === 'add' ? (resp.added ?? []) : (resp.removed ?? [])
  return (
    <Card>
      <MetaRow
        items={kv([
          ['id', resp.id],
          ['bytes', formatBytes(resp.bytes)],
          ['modified', ago(resp.modified_at)],
        ])}
      >
        <Badge variant={resp.unchanged ? 'default' : 'accent'}>
          {resp.unchanged ? 'unchanged' : verb === 'add' ? 'functions added' : 'functions removed'}
        </Badge>
      </MetaRow>
      <ActionLine icon={verb === 'add' ? <Plus /> : <Minus />}>
        <span className="dir-ui-identity">
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
        </span>
      </ActionLine>
    </Card>
  )
}
