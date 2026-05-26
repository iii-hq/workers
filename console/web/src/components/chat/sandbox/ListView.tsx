import { Inbox } from 'lucide-react'
import { EmptyState } from '@/components/ui/EmptyState'
import { StatusDot } from '@/components/ui/StatusDot'
import { formatAgeSecs, truncateMiddle } from './format'
import { listResponseSchema, safeParseResponse } from './parsers'

interface ListViewProps {
  output: unknown
}

export function ListView({ output }: ListViewProps) {
  const parsed = safeParseResponse(listResponseSchema, output)
  if (!parsed) return null
  const sandboxes = parsed.sandboxes

  if (sandboxes.length === 0) {
    return (
      <div className="border-t border-rule-2 bg-bg p-3">
        <EmptyState
          icon={Inbox}
          title="no sandboxes"
          description="no live sandboxes for this worker."
        />
      </div>
    )
  }

  return (
    <div className="border-t border-rule-2 bg-bg overflow-x-auto">
      <table className="w-full font-mono text-[12px] text-ink">
        <thead>
          <tr className="bg-paper-2 border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            <th className="text-left font-normal px-3 py-1.5">sandbox</th>
            <th className="text-left font-normal px-2 py-1.5">name</th>
            <th className="text-left font-normal px-2 py-1.5">image</th>
            <th className="text-left font-normal px-2 py-1.5">age</th>
            <th className="text-left font-normal px-2 py-1.5">exec</th>
            <th className="text-left font-normal px-3 py-1.5">state</th>
          </tr>
        </thead>
        <tbody>
          {sandboxes.map((s) => (
            <tr
              key={s.sandbox_id}
              className="border-b border-rule-2 last:border-b-0"
            >
              <td className="px-3 py-1.5">
                <code className="text-ink">
                  {truncateMiddle(s.sandbox_id, 14)}
                </code>
              </td>
              <td className="px-2 py-1.5 text-ink-faint">{s.name ?? '—'}</td>
              <td className="px-2 py-1.5 text-ink-faint">{s.image}</td>
              <td className="px-2 py-1.5 text-ink-faint tabular-nums">
                {formatAgeSecs(s.age_secs)}
              </td>
              <td className="px-2 py-1.5">
                <StatusDot
                  tone={s.exec_in_progress ? 'accent' : 'ink'}
                  pulse={s.exec_in_progress}
                />
              </td>
              <td className="px-3 py-1.5">
                {s.stopped ? (
                  <span className="text-warn">stopped</span>
                ) : (
                  <span className="text-ink-faint">live</span>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
