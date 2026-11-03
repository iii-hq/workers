import { ActionLine, Badge, Card, MetaRow } from '@iii-dev/console-ui'
import { Download } from 'lucide-react'
import { kv, Loading, Row, Rows, Section } from '../lib/widgets'
import {
  type SkillsDownloadRequest,
  safeParseRequest,
  safeParseResponse,
  skillsDownloadRequestSchema,
  skillsDownloadResponseSchema,
} from './parsers'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

type Source =
  | { kind: 'repo'; repo: string; skill: string; branch: string }
  | { kind: 'registry'; worker: string; spec: string }

/** Classifies the request shape into the two valid source modes (repo or
 * registry). Only the active source's chips render — `null` means the
 * request was malformed and the dispatcher will fall back to JSON. */
function classifySource(req: SkillsDownloadRequest): Source | null {
  if (req.repo && req.skill) {
    return { kind: 'repo', repo: req.repo, skill: req.skill, branch: req.branch ?? 'main' }
  }
  if (req.worker) {
    const spec = req.version ? `v${req.version}` : req.tag ? `${req.tag}` : 'latest'
    return { kind: 'registry', worker: req.worker, spec }
  }
  return null
}

const sourceItems = (source: Source) =>
  source.kind === 'repo'
    ? kv([
        ['Source', 'Repo'],
        ['branch', source.branch],
      ])
    : kv([
        ['Source', 'Registry'],
        ['spec', source.spec],
      ])

const describeSource = (source: Source) =>
  source.kind === 'repo'
    ? `${source.repo} › skills/${source.skill}@${source.branch}`
    : `registry: ${source.worker}@${source.spec}`

export function SkillsDownloadView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsDownloadRequestSchema, input)
  if (!req) return null
  const source = classifySource(req)
  if (!source) return null

  if (running) {
    return (
      <Card>
        <MetaRow items={sourceItems(source)}>
          <Badge>downloading…</Badge>
        </MetaRow>
        <ActionLine icon={<Download />} tone="ink">
          {describeSource(source)}
        </ActionLine>
        <Loading label="cloning + writing skills…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsDownloadResponseSchema, output)
  if (!resp) return null

  return (
    <Card>
      <MetaRow
        items={kv([
          ['namespace', resp.namespace],
          ['skills', resp.skills_written.length],
        ])}
      >
        <Badge variant="accent">downloaded</Badge>
      </MetaRow>
      <ActionLine icon={<Download />} tone="ink">
        {describeSource(source)}
      </ActionLine>
      <WrittenList label="skills written" names={resp.skills_written} />
      <WrittenList label="system prompts written" names={resp.system_prompts_written} />
      <WrittenList label="agent profiles written" names={resp.agents_written} />
    </Card>
  )
}

function WrittenList({ label, names }: { label: string; names: string[] }) {
  return (
    <Section label={`${label} · ${names.length}`}>
      {names.length === 0 ? (
        <div className="dir-ui-empty">none</div>
      ) : (
        <Rows>
          {names.map((n) => (
            <Row key={n} mono title={n} />
          ))}
        </Rows>
      )}
    </Section>
  )
}
