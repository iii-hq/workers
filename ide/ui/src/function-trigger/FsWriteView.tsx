import { formatBytes } from '../lib/format'
import { Chip, FooterPill } from '../lib/terminal'
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
    <div className="shui-card">
      <div className="shui-slab">
        <div className="shui-line">
          <span className="t-accent">+ wrote</span>{' '}
          <span className="num">{formatBytes(resp.bytes_written)}</span>{' '}
          <span className="t-faint">to</span>{' '}
          <span>{displayPath(req.data.path ?? '', resp.path)}</span>
        </div>
        <div className="shui-row">
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
    <div className="shui-card">
      <div className="shui-head">
        <span className="shui-chips">
          <Chip label="files">{resp.files.length}</Chip>
          <TargetChip target={req.target} />
        </span>
      </div>

      <table className="shui-table">
        <thead>
          <tr>
            <th className="pad-l">path</th>
            <th className="r">bytes</th>
            <th>mode</th>
            <th className="pad-r">content</th>
          </tr>
        </thead>
        <tbody>
          {resp.files.map((r, i) => {
            const spec = specByPath.get(r.path) ?? specs[i]
            return (
              <tr key={r.path}>
                <td className="pad-l t-ink">{r.path}</td>
                <td className="t-faint num r">{formatBytes(r.bytes_written)}</td>
                <td className="t-faint">
                  {spec ? (spec.mode ?? '0644') : '—'}
                  {spec?.parents ? <span className="t-ghost"> +parents</span> : null}
                </td>
                <td className="t-faint pad-r">
                  {spec
                    ? typeof spec.content === 'string'
                      ? 'inline'
                      : 'channel'
                    : '—'}
                </td>
              </tr>
            )
          })}
          <tr className="total">
            <td className="pad-l label">total</td>
            <td colSpan={3}>
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
