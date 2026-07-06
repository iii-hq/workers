/**
 * Post-write `checks` rows — shared by `coder::create-file`,
 * `coder::update-file` and `coder::apply-patch` responses. One compact
 * row per check: monospace command, a status badge, and the output
 * behind a native `<details>` disclosure. `error` set (e.g. a timeout)
 * means the exit code is not trustworthy — it wins over `exit_code`
 * and renders amber, never as a plain failure exit.
 */
import { FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import type { CheckResult } from './parsers'

export interface CheckStatus {
  label: string
  tone: 'accent' | 'warn' | 'alert' | 'default'
}

/** Badge state for one check: error → amber, exit 0 → green, non-zero →
    red, no exit code at all → neutral dash. */
export function checkStatus(check: CheckResult): CheckStatus {
  if (check.error != null && check.error !== '') {
    return { label: check.error, tone: 'warn' }
  }
  if (check.exit_code == null) {
    return { label: '—', tone: 'default' }
  }
  if (check.exit_code === 0) {
    return { label: 'exit 0', tone: 'accent' }
  }
  return { label: `exit ${check.exit_code}`, tone: 'alert' }
}

function CheckRow({ check }: { check: CheckResult }) {
  const status = checkStatus(check)
  const hasOutput = check.output !== ''

  return (
    <details className="border-b border-rule-2 last:border-b-0">
      <summary className="px-3 py-1.5 flex flex-wrap items-center gap-2 cursor-pointer list-none select-none">
        <span className="font-mono text-[12px] text-ink break-all">
          {check.command}
        </span>
        <FooterPill tone={status.tone}>{status.label}</FooterPill>
        {check.truncated ? (
          <FooterPill tone="warn">output truncated</FooterPill>
        ) : null}
      </summary>
      {hasOutput ? (
        <pre className="px-3 pb-2 font-mono text-[12px] leading-[1.55] text-ink whitespace-pre-wrap break-all">
          {check.output}
        </pre>
      ) : (
        <div className="px-3 pb-2 font-mono text-[12px] text-ink-ghost">
          · no output
        </div>
      )}
    </details>
  )
}

/** Renders nothing when `checks` is absent or empty. */
export function ChecksList({
  checks,
}: {
  checks?: readonly CheckResult[] | null
}) {
  if (!checks || checks.length === 0) return null
  return (
    <div className="border-t border-rule-2">
      <div className="px-3 pt-2 pb-1 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        checks
      </div>
      {checks.map((check, i) => (
        <CheckRow
          /* Commands can repeat (e.g. the same test run twice) — the
             ordinal disambiguates; checks are a static wire snapshot. */
          // biome-ignore lint/suspicious/noArrayIndexKey: checks never reorder
          key={`${check.command}:${i}`}
          check={check}
        />
      ))}
    </div>
  )
}
