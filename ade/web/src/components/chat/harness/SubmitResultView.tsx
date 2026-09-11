import { JsonHighlight } from '@/lib/syntax'

interface SubmitResultViewProps {
  /** The call ARGUMENTS — which for `submit_result` ARE the deliverable. */
  input: unknown
  running?: boolean
}

function isEmpty(v: unknown): boolean {
  if (v == null) return true
  if (typeof v === 'string') return v.length === 0
  if (Array.isArray(v)) return v.length === 0
  if (typeof v === 'object') return Object.keys(v as object).length === 0
  return false
}

/**
 * `submit_result` — the harness's synthetic output-contract tool. The call's
 * ARGUMENTS are the turn's deliverable (the agent's structured result); the
 * harness consumes the call and it has no response. So we render the submitted
 * result as the focus and never show the (always-empty) response pane that
 * made the generic view read as broken.
 */
export function SubmitResultView({ input, running }: SubmitResultViewProps) {
  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {running ? 'submitting result…' : 'submitted result'}
      </div>
      {isEmpty(input) ? (
        <div className="px-3 py-2 font-mono text-[11.5px] text-ink-ghost">
          · no result
        </div>
      ) : typeof input === 'string' ? (
        <pre className="bg-bg overflow-auto max-h-80 px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
          {input}
        </pre>
      ) : (
        <JsonHighlight
          code={JSON.stringify(input, null, 2)}
          className="max-h-80 overflow-auto"
          wrap
        />
      )}
    </div>
  )
}
