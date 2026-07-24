import type { ReactNode } from 'react'
import {
  ActionLine,
  Card,
  EmptyRow,
  KvChip,
  MetaRow,
  PulseLine,
  SectionHead,
  StatusPill,
} from '../lib/widgets'
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

/** Classifies the request shape into the two valid source modes (repo or
 * registry). Only the active source's chips render — `null` means the
 * request was malformed and the dispatcher will fall back to JSON. */
function classifySource(req: SkillsDownloadRequest):
  | { kind: 'repo'; repo: string; skill: string; branch: string }
  | { kind: 'registry'; worker: string; spec: string }
  | null {
  if (req.repo && req.skill) {
    return {
      kind: 'repo',
      repo: req.repo,
      skill: req.skill,
      branch: req.branch ?? 'main',
    }
  }
  if (req.worker) {
    const spec = req.version
      ? `v${req.version}`
      : req.tag
        ? `${req.tag}`
        : 'latest'
    return { kind: 'registry', worker: req.worker, spec }
  }
  return null
}

export function SkillsDownloadView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(skillsDownloadRequestSchema, input)
  if (!req) return null
  const source = classifySource(req)
  if (!source) return null

  if (running) {
    return (
      <Card>
        <MetaRow>
          <StatusPill label="downloading…" variant="default" />
          {sourceChips(source)}
        </MetaRow>
        <ActionLine symbol="↓" tone="ink">
          <span>{describeSource(source)}</span>
        </ActionLine>
        <PulseLine label="cloning + writing skills…" />
      </Card>
    )
  }

  const resp = safeParseResponse(skillsDownloadResponseSchema, output)
  if (!resp) return null

  const skillsCount = resp.skills_written.length
  const promptsCount = resp.prompts_written.length

  return (
    <Card>
      <MetaRow>
        <StatusPill label="downloaded" variant="accent" />
        <KvChip label="namespace">{resp.namespace}</KvChip>
        <KvChip label="skills">{skillsCount}</KvChip>
        <KvChip label="prompts">{promptsCount}</KvChip>
      </MetaRow>
      <ActionLine symbol="↓" tone="ink">
        <span>{describeSource(source)}</span>
      </ActionLine>
      <WrittenList label="skills written" names={resp.skills_written} />
      <WrittenList label="prompts written" names={resp.prompts_written} />
    </Card>
  )
}

function WrittenList({ label, names }: { label: string; names: string[] }) {
  return (
    <div className="dir-ui-section">
      <SectionHead>
        {label} · {names.length}
      </SectionHead>
      {names.length === 0 ? (
        <EmptyRow label="none" />
      ) : (
        <ul className="dir-ui-list">
          {names.map((n) => (
            <li key={n} className="dir-ui-item">
              {n}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function sourceChips(
  source: ReturnType<typeof classifySource> & object,
): ReactNode {
  if (source.kind === 'repo') {
    return (
      <>
        <KvChip label="source">repo</KvChip>
        <KvChip label="branch">{source.branch}</KvChip>
      </>
    )
  }
  return (
    <>
      <KvChip label="source">registry</KvChip>
      <KvChip label="spec">{source.spec}</KvChip>
    </>
  )
}

function describeSource(
  source: ReturnType<typeof classifySource> & object,
): string {
  if (source.kind === 'repo') {
    return `${source.repo} › skills/${source.skill}@${source.branch}`
  }
  return `registry: ${source.worker}@${source.spec}`
}
