import { formatBytes } from '@/components/chat/sandbox/format'
import { Chip, FooterPill } from '@/components/chat/sandbox/terminal/Terminal'
import {
  contentRefSchema,
  type FsWriteRequest,
  type FsWriteResponse,
  fsWriteRequestSchema,
  fsWriteResponseSchema,
  safeParseResponse,
} from './parsers'
import { displayPath, TargetChip } from './shared'

interface FsWriteViewProps {
  input: unknown
  output: unknown
}

export function FsWriteView({ input, output }: FsWriteViewProps) {
  const req = fsWriteRequestSchema.safeParse(input)
  if (!req.success) return null
  const resp = safeParseResponse(fsWriteResponseSchema, output)
  if (!resp) return null

  /* Request-side discriminator: batch when `files` is a non-empty array.
     (The response-side `path === ''` heuristic is unreliable —
     sandbox-target single writes blank it too.) */
  if (req.data.files?.length) {
    return <BatchWrite req={req.data} resp={resp} />
  }

  const streamed =
    req.data.content != null &&
    contentRefSchema.safeParse(req.data.content).success

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="px-3 py-3 flex flex-col gap-2">
        <div className="font-mono text-[12.5px] text-ink">
          <span className="text-accent">+ wrote</span>{' '}
          <span className="tabular-nums">
            {formatBytes(resp.bytes_written)}
          </span>{' '}
          <span className="text-ink-faint">to</span>{' '}
          <span>{displayPath(req.data.path ?? '', resp.path)}</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip label="mode">{req.data.mode ?? '0644'}</Chip>
          {req.data.parents ? <Chip label="parents">true</Chip> : null}
          {streamed ? (
            <FooterPill tone="default">uploaded via channel</FooterPill>
          ) : null}
          <TargetChip target={req.data.target} />
        </div>
      </div>
    </div>
  )
}

interface BatchWriteProps {
  req: FsWriteRequest
  resp: FsWriteResponse
}

/** Batch layout — one row per written file. `resp.files` is authoritative
    (worker preserves order); each row joins back to its request spec by
    path (index fallback) for mode/content provenance. */
function BatchWrite({ req, resp }: BatchWriteProps) {
  const specs = req.files ?? []
  const specByPath = new Map(specs.map((s) => [s.path, s] as const))

  return (
    <div className="border-t border-rule-2 bg-bg">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="files">{resp.files.length}</Chip>
        <TargetChip target={req.target} />
      </div>

      <table className="w-full font-mono text-[12px] text-ink">
        <thead>
          <tr className="border-b border-rule-2 text-[11px] uppercase tracking-[0.06em] text-ink-faint">
            <th className="text-left font-normal px-3 py-1.5">path</th>
            <th className="text-right font-normal px-2 py-1.5 w-20">bytes</th>
            <th className="text-left font-normal px-2 py-1.5 w-20">mode</th>
            <th className="text-left font-normal px-3 py-1.5 w-16">content</th>
          </tr>
        </thead>
        <tbody>
          {resp.files.map((r, i) => {
            const spec = specByPath.get(r.path) ?? specs[i]
            return (
              <tr
                key={r.path}
                className="border-b border-rule-2 last:border-b-0"
              >
                <td className="px-3 py-1.5 text-ink">{r.path}</td>
                <td className="px-2 py-1.5 text-right text-ink-faint tabular-nums">
                  {formatBytes(r.bytes_written)}
                </td>
                <td className="px-2 py-1.5 text-ink-faint">
                  {spec ? (spec.mode ?? '0644') : '—'}
                  {spec?.parents ? (
                    <span className="text-ink-ghost"> +parents</span>
                  ) : null}
                </td>
                <td className="px-3 py-1.5 text-ink-faint">
                  {spec
                    ? typeof spec.content === 'string'
                      ? 'inline'
                      : 'channel'
                    : '—'}
                </td>
              </tr>
            )
          })}
          <tr className="bg-paper-2">
            <td className="px-3 py-1.5 text-ink-faint uppercase tracking-[0.06em] text-[11px]">
              total
            </td>
            <td className="px-2 py-1.5" colSpan={3}>
              <FooterPill tone="accent">
                {`${formatBytes(resp.bytes_written)} · ${resp.files.length} ${resp.files.length === 1 ? 'file' : 'files'}`}
              </FooterPill>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  )
}
