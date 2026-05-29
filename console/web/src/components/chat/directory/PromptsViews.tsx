import {
  ActionLine,
  MetaRow,
  StatusPill,
} from '@/components/chat/sandbox/shared'
import {
  promptsGetRequestSchema,
  promptsGetResponseSchema,
  promptsListResponseSchema,
  safeParseRequest,
  safeParseResponse,
} from './parsers'
import { formatRelativeTime, KvChip, MarkdownPane } from './shared'

interface ViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

/* ---------------- directory::prompts::list ---------------- */

export function PromptsListView({ output, running }: ViewProps) {
  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="listing…" variant="default" />
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · scanning prompts folder…
        </div>
      </div>
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
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill
          label={label}
          variant={resp.prompts.length === 0 ? 'warn' : 'accent'}
        />
      </MetaRow>
      {resp.prompts.length === 0 ? (
        <div className="px-3 py-4 font-mono text-[12.5px] text-ink-ghost">
          · no prompts found
        </div>
      ) : (
        <ul className="divide-y divide-rule-2">
          {resp.prompts.map((p) => (
            <li key={p.name} className="px-3 py-2 flex flex-col gap-0.5">
              <span className="font-mono text-[12.5px] text-accent break-all">
                {p.name}
              </span>
              {p.description ? (
                <div className="font-mono text-[12px] text-ink-faint leading-[1.55]">
                  {p.description}
                </div>
              ) : null}
              <span className="font-mono text-[11px] text-ink-ghost">
                {formatRelativeTime(p.modified_at)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

/* ---------------- directory::prompts::get ---------------- */

export function PromptsGetView({ input, output, running }: ViewProps) {
  const req = safeParseRequest(promptsGetRequestSchema, input)

  if (running) {
    return (
      <div className="border-t border-rule-2 bg-bg">
        <MetaRow>
          <StatusPill label="loading…" variant="default" />
          {req ? <KvChip label="name">{req.name}</KvChip> : null}
        </MetaRow>
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · fetching prompt…
        </div>
      </div>
    )
  }

  const resp = safeParseResponse(promptsGetResponseSchema, output)
  if (!resp) return null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        <StatusPill label="prompt" variant="accent" />
        <KvChip label="modified">{formatRelativeTime(resp.modified_at)}</KvChip>
      </MetaRow>
      <ActionLine symbol="ƒ" tone="accent">
        <div className="flex flex-col">
          <span className="font-mono text-[13px] text-accent break-all">
            {resp.name}
          </span>
          {resp.description ? (
            <span className="font-mono text-[11.5px] text-ink-faint">
              {resp.description}
            </span>
          ) : null}
        </div>
      </ActionLine>
      <MarkdownPane body={resp.body} />
    </div>
  )
}
