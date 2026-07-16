import { FilterChip } from '@/components/chat/engine/shared'
import { MetaRow, StatusPill } from '@/components/chat/sandbox/shared'
import {
  type PipeResponse,
  pipeRequestSchema,
  pipeResponseSchema,
  safeParseRequest,
} from './parsers'

interface PipeViewProps {
  input: unknown
  /** Already unwrapped once by the dispatcher — never unwrap again. */
  output?: unknown
  running?: boolean
}

/** Single-line payload summary under each step; empty payloads render
    nothing (the raw json tab keeps the full call). */
function compactPayload(payload: unknown): string | null {
  if (payload == null) return null
  if (
    typeof payload === 'object' &&
    !Array.isArray(payload) &&
    Object.keys(payload as Record<string, unknown>).length === 0
  ) {
    return null
  }
  return JSON.stringify(payload)
}

/** Dim any `ns::` prefix so the op tail reads clearly (`state::set`,
    `fp::get`), mirroring the header id labels. */
function StepFunctionLabel({ functionId }: { functionId: string }) {
  const splitAt = functionId.lastIndexOf('::')
  if (splitAt <= 0) return <span className="text-ink">{functionId}</span>
  return (
    <>
      <span className="text-ink-faint">{functionId.slice(0, splitAt + 2)}</span>
      <span className="text-ink">{functionId.slice(splitAt + 2)}</span>
    </>
  )
}

/** `fp::pipe` — the step route with per-step receipt sizes, then the
    final value preview. The threaded values themselves never reach the chat
    (that is the point of the pipe), so there is no value pane. */
export function PipeView({ input, output, running }: PipeViewProps) {
  const req = safeParseRequest(pipeRequestSchema, input)
  if (!req?.through?.length) return null

  const parsed = running ? null : pipeResponseSchema.safeParse(output)
  const res: PipeResponse | null = parsed?.success ? parsed.data : null
  const receipts = res?.steps ?? null
  const threaded = receipts?.length
    ? receipts[receipts.length - 1]?.chars
    : null

  return (
    <div className="border-t border-rule-2 bg-bg">
      <MetaRow>
        {/* only claim ok once receipts exist — approval previews render
            this view before the pipe has run */}
        <StatusPill
          label={running ? 'piping…' : receipts ? 'pipe ok' : 'pipe'}
          variant={!running && receipts ? 'accent' : 'default'}
        />
        <FilterChip label="steps" value={String(req.through.length)} />
        {threaded != null ? (
          <FilterChip
            label="threaded"
            value={`${threaded.toLocaleString()} chars`}
          />
        ) : null}
      </MetaRow>
      <ul className="divide-y divide-rule-2">
        {req.through.map((step, i) => {
          const receipt = receipts?.[i]
          const payload = compactPayload(step.payload)
          return (
            <li
              key={`${i}-${step.function}`}
              className="px-3 py-1.5 font-mono text-[12.5px]"
            >
              <div className="flex items-center gap-2">
                <span className="w-4 shrink-0 text-right text-ink-ghost">
                  {i + 1}
                </span>
                <span className="min-w-0 truncate">
                  <StepFunctionLabel functionId={step.function} />
                </span>
                {step.into ? (
                  <FilterChip label="into" value={step.into} />
                ) : null}
                {receipt?.chars != null ? (
                  <span className="ml-auto shrink-0 text-[11px] text-ink-faint">
                    → {receipt.chars.toLocaleString()} chars
                  </span>
                ) : null}
              </div>
              {payload ? (
                <div className="mt-0.5 break-all pl-6 text-[11px] leading-[1.5] text-ink-faint line-clamp-2">
                  {payload}
                </div>
              ) : null}
            </li>
          )
        })}
      </ul>
      {running ? (
        <div className="px-3 py-3 font-mono text-[12.5px] text-ink-ghost animate-pulse">
          · piping…
        </div>
      ) : res?.value_preview ? (
        <div>
          <div className="px-3 pt-2 font-mono text-[10px] uppercase tracking-[0.06em] text-ink-ghost">
            preview
          </div>
          <pre className="overflow-x-auto whitespace-pre-wrap break-all px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink">
            {res.value_preview}
          </pre>
        </div>
      ) : null}
    </div>
  )
}
