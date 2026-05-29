import type { ReactNode } from 'react'
import {
  ActionLine,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  type SkillsDownloadRequest,
  safeParseRequest,
  safeParseResponse,
  skillsDownloadRequestSchema,
  skillsDownloadResponseSchema,
} from './parsers'
import { KvChip } from './shared'

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
  | {
      kind: 'registry'
      worker: string
      spec: string
    }
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
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="downloading…" variant="default" />
          {sourceChips(source)}
        </MetaRow>
        <ActionLine symbol="↓" tone="ink">
          <span className="break-all">{describeSource(source)}</span>
        </ActionLine>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · cloning + writing skills…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(skillsDownloadResponseSchema, output)
  if (!resp) return null

  const skillsCount = resp.skills_written.length
  const promptsCount = resp.prompts_written.length

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="downloaded" variant="accent" />
        <KvChip label="namespace">{resp.namespace}</KvChip>
        <KvChip label="skills">{skillsCount}</KvChip>
        <KvChip label="prompts">{promptsCount}</KvChip>
      </MetaRow>
      <ActionLine symbol="↓" tone="ink">
        <span className="break-all">{describeSource(source)}</span>
      </ActionLine>
      <WrittenList label="skills written" names={resp.skills_written} />
      <WrittenList label="prompts written" names={resp.prompts_written} />
    </div>
  )
}

function WrittenList({ label, names }: { label: string; names: string[] }) {
  return (
    <div className="border-b border-rule-2">
      <div className="px-3 py-1.5 bg-paper-2 border-b border-rule-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-faint">
        {label} · {names.length}
      </div>
      {names.length === 0 ? (
        <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
          · none
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {names.map((n) => (
            <li
              key={n}
              className="px-3 py-1 font-mono text-[11.5px] text-ink break-all"
            >
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
